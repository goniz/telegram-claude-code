use bollard::Docker;
use tokio::sync::{mpsc, oneshot};

use super::{config::ClaudeCodeConfig, container_cred_storage::ContainerCredStorage};
use crate::oauth::{ClaudeAuth, Config as OAuthConfig, Credentials};

#[derive(Debug, Clone, PartialEq)]
pub enum AuthState {
    Starting,
    UrlReady(String),
    WaitingForCode,
    Completed(String),
    Failed(String),
}

#[derive(Debug)]
pub struct AuthenticationHandle {
    pub state_receiver: mpsc::UnboundedReceiver<AuthState>,
    pub code_sender: mpsc::UnboundedSender<String>,
    pub cancel_sender: oneshot::Sender<()>,
}

/// Authentication functionality for Claude Code client
#[derive(Debug)]
pub struct AuthenticationManager {
    docker: Docker,
    container_id: String,
    oauth_client: ClaudeAuth,
}

impl AuthenticationManager {
    pub fn new(docker: Docker, container_id: String, config: ClaudeCodeConfig) -> Self {
        let storage = Box::new(ContainerCredStorage::new(
            docker.clone(),
            container_id.clone(),
        ));
        let oauth_client = ClaudeAuth::with_custom_storage(config.oauth_config.clone(), storage);

        Self {
            docker,
            container_id,
            oauth_client,
        }
    }

    /// Authenticate Claude Code using OAuth 2.0 flow
    /// Returns an AuthenticationHandle for channel-based communication
    pub async fn authenticate_claude_account(
        &self,
    ) -> Result<AuthenticationHandle, Box<dyn std::error::Error + Send + Sync>> {
        // Check if valid credentials already exist
        match self.oauth_client.load_credentials().await {
            Ok(Some(credentials)) if !credentials.is_expired() => {
                // Check if credentials work with Claude Code
                if let Ok(true) = self.check_oauth_credentials(&credentials).await {
                    let (state_sender, state_receiver) = mpsc::unbounded_channel();
                    let (code_sender, _code_receiver) = mpsc::unbounded_channel();
                    let (cancel_sender, _cancel_receiver) = oneshot::channel();

                    // Send immediate completion state
                    let _ = state_sender.send(AuthState::Completed(
                        "✅ Claude Code is already authenticated and ready to use!".to_string(),
                    ));

                    return Ok(AuthenticationHandle {
                        state_receiver,
                        code_sender,
                        cancel_sender,
                    });
                }
            }
            _ => {
                // No valid credentials found, start OAuth flow
            }
        }

        // Start OAuth authentication flow
        self.start_oauth_flow().await
    }

    /// Check if OAuth credentials are valid by testing them with Claude Code
    async fn check_oauth_credentials(
        &self,
        _credentials: &Credentials,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Implement actual credential validation
        // For now, return true if credentials are not expired
        Ok(true)
    }

    /// Start OAuth authentication flow with channel communication
    async fn start_oauth_flow(
        &self,
    ) -> Result<AuthenticationHandle, Box<dyn std::error::Error + Send + Sync>> {
        let (state_sender, state_receiver) = mpsc::unbounded_channel();
        let (code_sender, code_receiver) = mpsc::unbounded_channel();
        let (cancel_sender, cancel_receiver) = oneshot::channel();

        // Clone necessary data for the background task
        let storage = Box::new(ContainerCredStorage::new(
            self.docker.clone(),
            self.container_id.clone(),
        ));
        let oauth_client = ClaudeAuth::with_custom_storage(OAuthConfig::default(), storage);
        let docker = self.docker.clone();
        let container_id = self.container_id.clone();

        // Spawn the background authentication task
        tokio::spawn(async move {
            let _ = Self::background_oauth_flow(
                oauth_client,
                docker,
                container_id,
                state_sender,
                code_receiver,
                cancel_receiver,
            )
            .await;
        });

        Ok(AuthenticationHandle {
            state_receiver,
            code_sender,
            cancel_sender,
        })
    }

    /// Background task for OAuth authentication flow
    async fn background_oauth_flow(
        oauth_client: ClaudeAuth,
        _docker: Docker,
        _container_id: String,
        state_sender: mpsc::UnboundedSender<AuthState>,
        mut code_receiver: mpsc::UnboundedReceiver<String>,
        cancel_receiver: oneshot::Receiver<()>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Duration;

        // Send starting state
        let _ = state_sender.send(AuthState::Starting);

        // Generate OAuth login URL
        let login_url = match oauth_client.generate_login_url().await {
            Ok(url) => {
                log::info!("Generated OAuth login URL: {}", url);
                url
            }
            Err(e) => {
                log::error!("Failed to generate OAuth login URL: {}", e);
                let _ = state_sender.send(AuthState::Failed(format!(
                    "Failed to generate login URL: {}",
                    e
                )));
                return Err(e.into());
            }
        };

        // Send URL to user
        let _ = state_sender.send(AuthState::UrlReady(login_url));
        let _ = state_sender.send(AuthState::WaitingForCode);

        // Wait for authorization code with timeout
        let timeout_result = tokio::time::timeout(Duration::from_secs(300), async {
            tokio::select! {
                // Handle cancellation
                _ = cancel_receiver => {
                    log::info!("OAuth authentication cancelled by user");
                    let _ = state_sender.send(AuthState::Failed("Authentication cancelled".to_string()));
                    return Ok(());
                }

                // Handle auth code input
                code = code_receiver.recv() => {
                    if let Some(auth_code) = code {
                        log::info!("Received authorization code from user");

                        // Exchange authorization code for tokens
                        match oauth_client.exchange_code(&auth_code).await {
                            Ok(credentials) => {
                                log::info!("Successfully obtained OAuth credentials");

                                // Save credentials to file
                                if let Err(e) = oauth_client.save_credentials(&credentials).await {
                                    log::warn!("Failed to save credentials: {}", e);
                                }

                                let _ = oauth_client.cleanup_state().await;
                                let _ = state_sender.send(AuthState::Completed(
                                    "✅ Claude Code authentication completed successfully!".to_string()
                                ));
                            }
                            Err(e) => {
                                log::error!("Failed to exchange authorization code: {}", e);
                                let _ = state_sender.send(AuthState::Failed(
                                    format!("Failed to exchange authorization code: {}", e)
                                ));
                            }
                        }
                    } else {
                        let _ = state_sender.send(AuthState::Failed("No authorization code received".to_string()));
                    }
                }
            }
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }).await;

        match timeout_result {
            Ok(Ok(())) => {
                log::info!("OAuth authentication completed successfully");
            }
            Ok(Err(e)) => {
                log::error!("Error during OAuth authentication: {}", e);
                let _ =
                    state_sender.send(AuthState::Failed(format!("Authentication error: {}", e)));
            }
            Err(_) => {
                log::warn!("OAuth authentication timed out after 5 minutes");
                let _ = state_sender.send(AuthState::Failed(
                    "Authentication timed out after 5 minutes".to_string(),
                ));
            }
        }

        Ok(())
    }
}
