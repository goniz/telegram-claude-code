pub mod auth;
pub mod operations;
pub mod types;

pub use auth::{GitHubAuth, OAuthProcess};
pub use operations::GitHubOperations;
pub use types::{GithubAuthResult, GithubClientConfig, GithubCloneResult};

use bollard::Docker;

/// Main GitHub client that combines authentication and operations
#[derive(Debug)]
pub struct GithubClient {
    auth: GitHubAuth,
    operations: GitHubOperations,
}

impl GithubClient {
    /// Create a new GitHub client for the specified container
    pub fn new(docker: Docker, container_id: String, config: GithubClientConfig) -> Self {
        let auth = GitHubAuth::new(docker.clone(), container_id.clone(), config.clone());
        let operations = GitHubOperations::new(docker, container_id, config);

        Self { auth, operations }
    }

    /// Get the container ID from the operations module
    #[allow(dead_code)]
    pub fn container_id(&self) -> &str {
        self.operations.container_id()
    }

    /// Authenticate with GitHub using gh client
    pub async fn login(
        &self,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        self.auth.login().await
    }

    /// Check GitHub authentication status
    pub async fn check_auth_status(
        &self,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        self.auth.check_auth_status().await
    }

    /// Logout from GitHub by removing stored credentials
    pub async fn logout(
        &self,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        self.auth.logout().await
    }

    /// Wait for OAuth completion after user has visited the URL
    #[allow(dead_code)]
    pub async fn wait_for_oauth_completion(
        &self,
        oauth_process: OAuthProcess,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        self.auth.wait_for_oauth_completion(oauth_process).await
    }

    /// Clone a repository using gh client
    pub async fn repo_clone(
        &self,
        repository: &str,
        target_dir: Option<&str>,
    ) -> Result<GithubCloneResult, Box<dyn std::error::Error + Send + Sync>> {
        self.operations.repo_clone(repository, target_dir).await
    }

    /// Check if gh client is available
    #[allow(dead_code)]
    pub async fn check_availability(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.operations.check_availability().await
    }

    /// List GitHub repositories for the authenticated user
    pub async fn repo_list(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.operations.repo_list().await
    }

    /// Helper method for basic command execution (used in tests)
    #[allow(dead_code)]
    pub async fn exec_basic_command(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.operations.exec_basic_command(command).await
    }

    /// Execute a command and return both output and success status (used in tests)
    #[allow(dead_code)]
    pub async fn exec_command_allow_failure(
        &self,
        command: Vec<String>,
    ) -> Result<(String, bool), Box<dyn std::error::Error + Send + Sync>> {
        self.operations.exec_command_allow_failure(command).await
    }
}
