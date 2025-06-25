# Test Suite Documentation

This document describes the comprehensive test suite for the Telegram Claude Code bot, including parallel execution safety and guaranteed cleanup patterns.

## Overview

The test suite is designed to ensure:
1. **Parallel execution safety** - Tests can run concurrently without interference
2. **Guaranteed cleanup** - Containers are always cleaned up, even on test failures or panics
3. **Resource isolation** - Each test gets its own unique container environment

### Core Test Files

1. **`claude_core_tests.rs`** - Core Claude functionality testing with new safe patterns
2. **`claude_authentication_tests.rs`** - Authentication workflow testing  
3. **`github_integration_tests.rs`** - GitHub integration testing with comprehensive clone functionality
4. **`infrastructure_tests.rs`** - Infrastructure and persistence testing
5. **`test_utils.rs`** - **NEW**: RAII-based test utilities for guaranteed cleanup

## Test Utilities (NEW)

### TestContainerGuard Pattern

The `TestContainerGuard` provides RAII-based container management with automatic cleanup:

```rust
use test_utils::TestContainerGuard;

#[tokio::test]
async fn test_with_auto_cleanup() -> test_utils::TestResult {
    let guard = TestContainerGuard::new().await?;
    let client = guard.start_coding_session().await?;
    
    // Test logic here - even if assertions fail or early returns happen,
    // the container will be cleaned up when guard goes out of scope
    
    // Optional explicit cleanup (recommended for critical tests)
    guard.cleanup().await;
    
    Ok(())
}
```

### Key Benefits

1. **Automatic Cleanup**: Containers are cleaned up even if tests panic or return early
2. **Unique Naming**: Each container gets a unique name with timestamp + UUID  
3. **Volume Management**: Persistent volumes are also cleaned up automatically
4. **Error Safety**: No more container leaks due to assertion failures

### Container Naming Strategy

Container names are generated using:
```
{prefix}-{timestamp_ms}-{uuid_simple}
```

This ensures:
- Parallel test execution without name conflicts
- Easy identification of test containers  
- Automatic uniqueness even with rapid test execution

Example: `test-1704123456789-a1b2c3d4e5f6g7h8`

## Test Categories

### 1. Basic Functionality Tests (Existing)

- **`test_github_client_creation`** - Verifies GitHub client can be created and configured properly
- **`test_gh_availability_check`** - Ensures GitHub CLI is available in the test environment
- **`test_github_auth_status_check`** - Tests authentication status checking
- **`test_github_basic_command_execution`** - Validates basic command execution through the client
- **`test_github_login_oauth_url_generation`** - Tests OAuth URL generation for authentication
- **`test_github_repo_list`** - Tests repository listing functionality
- **`test_github_client_working_directory_config`** - Tests custom working directory configuration

### 2. Clone Functionality Edge Cases (Newly Added)

#### Repository Type Testing
- **`test_github_repo_clone_empty_repository`** - Tests cloning empty repositories
- **`test_github_repo_clone_multi_branch_repository`** - Tests repositories with multiple branches
- **`test_github_repo_clone_large_repository`** - Tests large repository handling (timeout scenarios)

#### URL and Input Validation
- **`test_github_repo_clone_malformed_urls`** - Tests various malformed repository URL formats:
  - Missing owner or repository name
  - Double slashes, extra path components
  - Empty strings, spaces in names
  - Invalid characters, full URLs instead of owner/repo format
- **`test_github_repo_clone_case_sensitivity`** - Tests case sensitivity handling in repository names
- **`test_github_repo_clone_special_characters_and_unicode`** - Tests special characters in repository and target directory names

#### Target Directory Handling
- **`test_github_repo_clone_target_directory_edge_cases`** - Tests various target directory scenarios:
  - Current directory (`.`)
  - Relative paths (`./test-dir`)
  - Absolute paths (`/tmp/test-clone`)
  - Spaces in directory names
  - Special characters and very long names
- **`test_github_repo_clone_target_directory_default_behavior`** - Tests default target directory extraction logic

#### Error Handling and Invalid Repositories
- **`test_github_repo_clone_invalid_repo`** - Tests invalid repository names (existing)
- **`test_github_repo_clone_with_target_directory`** - Tests target directory specification (existing)
- **`test_github_repo_clone_nonexistent_variations`** - Tests different types of nonexistent repositories
- **`test_github_repo_clone_private_repo_without_auth`** - Tests private repository access without authentication

#### Concurrency and Performance
- **`test_github_repo_clone_concurrent_operations`** - Tests multiple concurrent clone operations

### 3. Integration Tests (Advanced Scenarios)

#### Configuration and Environment Testing
- **`test_github_clone_with_different_working_directories`** - Tests clone operations with various working directory configurations
- **`test_github_clone_with_different_timeouts`** - Tests different timeout configurations and their effects

#### Error Handling and Recovery
- **`test_github_clone_error_handling_and_recovery`** - Tests fail-succeed-fail patterns and recovery scenarios

#### Authentication Integration
- **`test_github_clone_authentication_scenarios`** - Tests clone operations with different authentication states

#### Repository Access Patterns
- **`test_github_clone_repository_access_patterns`** - Tests different types of repositories (small, large, active projects)

#### Complete Workflow Testing
- **`test_github_clone_workflow_integration`** - Tests complete workflow: availability check → auth check → repo list → clone operations

## Test Infrastructure

### Fixtures and Setup
- **Docker-based testing** - All tests run in isolated Docker containers
- **`test_container` fixture** - Provides clean container environment for each test
- **`cleanup_container` function** - Ensures proper cleanup after each test
- **`rstest` framework** - Enables parameterized and async testing

### Error Handling Approach
Tests are designed to handle authentication failures gracefully, as the test environment may not have GitHub authentication configured. Key principles:

1. **Always verify result structure** - Even failed operations should return properly structured results
2. **Test error messages** - Validate that error messages are informative and not empty
3. **Validate target directory logic** - Ensure target directory extraction works correctly even for failed operations
4. **Handle authentication scenarios** - Tests should work both with and without GitHub authentication

## Expected Behavior

### With Authentication
- Public repositories should clone successfully
- Private repositories (if accessible) should clone successfully
- Error messages should be specific to actual clone issues

### Without Authentication
- Public repositories may still work for some operations
- Most operations will fail with authentication errors
- Error messages should clearly indicate authentication requirements
- Target directory and repository name handling should still work correctly

## Running the Tests

### Run All GitHub Client Tests
```bash
cargo test --test github_client_tests
```

### Run Specific Edge Case Tests
```bash
cargo test --test github_client_tests test_github_repo_clone_malformed_urls -- --nocapture
cargo test --test github_client_tests test_github_repo_clone_target_directory_edge_cases -- --nocapture
```

### Run Integration Tests
```bash
cargo test --test github_clone_integration_test
```

### Run Individual Integration Tests
```bash
cargo test --test github_clone_integration_test test_github_clone_authentication_scenarios -- --nocapture
```

## Test Results and Validation

All tests validate:
1. **Method returns `Ok` result** - Clone operations should never panic or return `Err`
2. **Result structure integrity** - `GithubCloneResult` should have correct repository name and target directory
3. **Non-empty messages** - Error or success messages should always be present
4. **Target directory logic** - Default and explicit target directory handling should work correctly
5. **Error handling** - Various failure modes should be handled gracefully

## Future Enhancements

Potential areas for additional testing:
1. **Network failure scenarios** - Simulating network timeouts and failures
2. **Git-specific operations** - Testing actual git operations post-clone
3. **File system permissions** - Testing clone operations with various permission scenarios
4. **Large-scale concurrent testing** - Stress testing with many simultaneous operations
5. **Authentication workflow testing** - Testing the complete OAuth authentication flow