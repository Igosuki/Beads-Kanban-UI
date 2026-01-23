# PR Reviewer - Agentform Configuration

Automated PR review system using Claude (Anthropic) with human-in-the-loop approval.

## Overview

This agentform configuration provides intelligent PR reviews tailored for Next.js/React/TypeScript projects. It uses:

- **Claude Sonnet** for thorough code review
- **Claude Haiku** for quick PR classification
- **MCP GitHub server** for GitHub API access

## Setup

### 1. Install agentform

```bash
# Install agentform CLI
npm install -g agentform
```

### 2. Set Environment Variables

Create a `.env` file or export these variables:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
export GITHUB_PERSONAL_ACCESS_TOKEN="ghp_..."
```

The GitHub token needs these scopes:
- `repo` - Full control of private repositories
- `pull_request` - Read/write access to pull requests

### 3. Create Input File

```bash
cp input.example.yaml input.yaml
# Edit input.yaml with your PR details
```

## Usage

### Run a PR Review

```bash
# Review a specific PR
agentform run review_pr --input input.yaml

# Or pass inputs directly
agentform run review_pr \
  --var owner=your-org \
  --var repo=beads-kanban-ui \
  --var pull_number=42
```

### Dry Run (No Submissions)

```bash
agentform run review_pr --input input.yaml --dry-run
```

## Workflow

```
                    +------------+
                    | fetch_pr   |
                    +-----+------+
                          |
                    +-----v------+
                    | fetch_files|
                    +-----+------+
                          |
                    +-----v------+
                    |  classify  |
                    +-----+------+
                          |
              +-----------+-----------+
              |                       |
        TRIVIAL                 SIMPLE/COMPLEX
              |                       |
      +-------v-------+       +-------v-------+
      | auto_approve  |       |   analyze     |
      +-------+-------+       +-------+-------+
              |                       |
            [END]             +-------v-------+
                              |   approval    |
                              | (human gate)  |
                              +-------+-------+
                                      |
                        +-------------+-------------+
                        |                           |
                   APPROVED                    REJECTED
                        |                           |
                +-------v-------+           +-------v-------+
                | submit_review |           | end_rejected  |
                +-------+-------+           +-------+-------+
                        |                           |
                      [END]                       [END]
```

## Agents

### Reviewer Agent (Claude Sonnet)

Performs thorough code review focusing on:

1. **TypeScript Correctness** - Types, generics, annotations
2. **React Best Practices** - Hooks, Server Components, re-renders
3. **Accessibility (WCAG AA)** - Semantic HTML, ARIA, keyboard nav
4. **Performance** - Data fetching, memoization, bundle size
5. **Security** - XSS, CSRF, secrets exposure
6. **Code Style** - Naming, structure, organization

### Classifier Agent (Claude Haiku)

Quick classification of PR complexity:

| Level | Criteria | Action |
|-------|----------|--------|
| TRIVIAL | Docs, deps (minor/patch), formatting | Auto-approve |
| SIMPLE | Single file <50 lines, tests, config | Quick review |
| COMPLEX | Multi-file, features, API changes | Full review |

## Policies

| Policy | Max Cost | Max Calls | Timeout |
|--------|----------|-----------|---------|
| review_policy | $0.50 | 15 | 300s |
| triage_policy | $0.05 | 3 | 30s |

## Cost Estimates

| PR Type | Estimated Cost |
|---------|----------------|
| Trivial (auto-approve) | ~$0.01 |
| Simple (quick review) | ~$0.05-0.15 |
| Complex (full review) | ~$0.15-0.40 |

## Files

```
agentform/pr-reviewer/
├── 00-project.af      # Project metadata
├── 01-variables.af    # API keys and tokens
├── 02-providers.af    # Anthropic provider and models
├── 03-servers.af      # MCP GitHub server
├── 04-capabilities.af # GitHub API capabilities
├── 05-policies.af     # Cost and timeout limits
├── 06-agents.af       # Reviewer and classifier agents
├── 07-workflows.af    # Review workflow definition
├── input.example.yaml # Example input
└── README.md          # This file
```

## Troubleshooting

### "Rate limited" errors

The GitHub MCP server may hit rate limits. Solutions:
- Use a GitHub App token instead of PAT (higher limits)
- Add delays between reviews
- Use `--dry-run` for testing

### "Timeout" errors

Complex PRs may exceed the 300s timeout. Solutions:
- Increase `timeout_seconds` in `review_policy`
- Review fewer files per run
- Split large PRs

### "Cost exceeded" errors

Review stopped due to cost limits. Solutions:
- Increase `max_cost_usd_per_run` in policies
- Use `--dry-run` to estimate costs first
