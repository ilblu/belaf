# belaf API - CLI Endpoints

This document specifies all API endpoints the CLI needs to function without direct GitHub API access.

## Architecture

```
CLI ──► belaf API ──► GitHub App Installation Token ──► GitHub API
         │
         └── User authenticated via Device Flow
```

The CLI uses the belaf API token (from Device Flow) to authenticate.
The API uses the GitHub App Installation Token to make GitHub API calls on behalf of the repository.

---

## Implemented Endpoints

### Device Flow Authentication

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/auth/device/code` | POST | Request device + user code |
| `/api/auth/device/token` | POST | Poll for access token |

### CLI Endpoints

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/api/cli/check-installation` | GET | Bearer | Check if GitHub App installed on repo |
| `/api/cli/me` | GET | Bearer | Get current user info |
| `/api/cli/repos/:owner/:repo` | GET | Bearer | Get repository metadata |
| `/api/cli/repos/:owner/:repo/commits` | GET | Bearer | List commits for changelog |
| `/api/cli/repos/:owner/:repo/pulls` | GET | Bearer | List pull requests for changelog |
| `/api/cli/repos/:owner/:repo/pulls` | POST | Bearer | Create a pull request |
| `/api/cli/repos/:owner/:repo/git/refs` | GET | Bearer | List tags/branches |
| `/api/cli/repos/:owner/:repo/compare/:basehead` | GET | Bearer | Compare commits between refs |
| `/api/cli/releases/latest` | GET | - | Get latest CLI release version |

---

## Endpoint Details

### GET `/api/cli/repos/:owner/:repo`

Get repository metadata.

```typescript
// Response
{
  "full_name": "owner/repo",
  "default_branch": "main",
  "private": false,
  "installation_id": 12345
}
```

### GET `/api/cli/repos/:owner/:repo/commits`

List commits for changelog generation.

```typescript
// Query params
?ref=main          // branch or tag (optional)
?per_page=100      // pagination (max 100)
?page=1

// Response
{
  "commits": [
    {
      "sha": "abc123...",
      "message": "feat: add new feature",
      "author": {
        "login": "username",
        "name": "User Name"
      },
      "timestamp": "2024-01-15T10:30:00Z"
    }
  ],
  "has_more": true
}
```

### GET `/api/cli/repos/:owner/:repo/pulls`

List pull requests for changelog generation.

```typescript
// Query params
?state=closed      // "open", "closed", or "all" (default: closed)
?per_page=100      // pagination (max 100)
?page=1

// Response
{
  "pull_requests": [
    {
      "number": 42,
      "title": "feat: implement feature X",
      "merge_commit_sha": "def456...",
      "labels": ["enhancement", "breaking-change"],
      "merged_at": "2024-01-15T10:30:00Z"
    }
  ],
  "has_more": false
}
```

### POST `/api/cli/repos/:owner/:repo/pulls`

Create a pull request.

```typescript
// Request
{
  "title": "chore(release): my-crate v1.2.0",
  "head": "release/v1.2.0",
  "base": "main",
  "body": "## Release Preparation\n..."  // optional
}

// Response
{
  "number": 123,
  "html_url": "https://github.com/owner/repo/pull/123",
  "state": "open"
}
```

### GET `/api/cli/repos/:owner/:repo/git/refs`

List tags and branches (for version detection).

```typescript
// Query params
?type=tag          // "tag" or "branch" (default: tag)
?prefix=v          // filter by prefix (e.g., "v" for version tags)

// Response
{
  "refs": [
    {
      "ref": "refs/tags/v1.2.0",
      "sha": "abc123..."
    },
    {
      "ref": "refs/tags/cli-v1.0.0",
      "sha": "def456..."
    }
  ]
}
```

### GET `/api/cli/repos/:owner/:repo/compare/:basehead`

Compare two refs (for changelog between versions).

```typescript
// basehead format: "base...head" (e.g., "v1.0.0...v1.1.0")

// Response
{
  "ahead_by": 15,
  "behind_by": 0,
  "commits": [
    {
      "sha": "abc123...",
      "message": "feat: new feature",
      "author": {
        "login": "username"
      }
    }
  ]
}
```

### GET `/api/cli/releases/latest`

Get latest CLI release version (public, no auth needed).

```typescript
// Response
{
  "tag_name": "cli-v1.2.0",
  "version": "1.2.0",
  "html_url": "https://github.com/ilblu/belaf/releases/tag/cli-v1.2.0",
  "published_at": "2024-01-15T10:30:00Z"
}
```

---

## Error Responses

All endpoints return consistent error format:

```typescript
{
  "error": "Repository not found or GitHub App not installed"
}
```

Common HTTP status codes:
- `400 Bad Request` - Missing required parameters
- `401 Unauthorized` - Invalid/expired API token
- `403 Forbidden` - No access to this repository
- `404 Not Found` - Repository not found or GitHub App not installed
- `422 Unprocessable` - Invalid request data (e.g., PR already exists)

---

## GitHub App Permissions Required

The belaf GitHub App needs these permissions:

| Permission | Access | Used For |
|------------|--------|----------|
| `contents` | Read & Write | Commits, branches, tags |
| `pull_requests` | Read & Write | Create PRs, list PRs |
| `metadata` | Read | Repository info |

---

## Authentication Flow

```
1. User runs `belaf install`
2. CLI gets API token via Device Flow
3. CLI stores API token in keyring
4. All GitHub operations go through API
5. API uses GitHub App Installation Token
```
