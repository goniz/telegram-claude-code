---
description: Create a new feature branch and plan implementation
allowed-tools: Bash(git fetch:*), Bash(git checkout:*), Bash(git branch:*)
---

# New Feature Branch Creation

## Current Repository Status
- Current branch: !`git branch --show-current`
- Repository status: !`git status --porcelain`
- Remote branches: !`git branch -r`

## Branch Creation Process

### Step 1: Fetch Latest Changes
!`git fetch -p`

### Step 2: Feature Description
**Feature/Task Description:** $ARGUMENTS

### Step 3: Branch Naming
Based on the feature description "$ARGUMENTS", I'll create an appropriate branch name following the pattern:
- `feature/descriptive-name` for new features
- `fix/descriptive-name` for bug fixes
- `refactor/descriptive-name` for refactoring
- `chore/descriptive-name` for maintenance tasks

### Step 4: Create and Switch to New Branch
I'll now create and switch to a new branch based on the latest main branch:

!`git checkout -b BRANCH_NAME origin/main`

## Implementation Plan

Based on the feature description "$ARGUMENTS", here's a suggested implementation plan:

### Analysis
I'll analyze the current codebase to understand:
1. Existing architecture and patterns
2. Related components that might be affected
3. Testing strategies and frameworks in use
4. Documentation requirements

### Implementation Steps
1. **Research Phase**
   - Examine existing similar functionality
   - Identify dependencies and integration points
   - Review coding standards and conventions

2. **Design Phase**
   - Plan the architecture and approach
   - Identify files to be created or modified
   - Consider backward compatibility

3. **Implementation Phase**
   - Implement core functionality
   - Add comprehensive tests
   - Update documentation
   - Follow existing code patterns

4. **Validation Phase**
   - Run test suite
   - Perform manual testing
   - Check for linting/formatting issues
   - Verify all requirements are met

### Next Steps
Would you like me to:
1. Start with the research phase to understand the current codebase?
2. Begin implementing specific components?
3. Focus on a particular aspect of the feature?

Please let me know how you'd like to proceed with implementing "$ARGUMENTS".