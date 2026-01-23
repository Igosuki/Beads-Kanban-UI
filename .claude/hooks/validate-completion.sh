#!/bin/bash
#
# SubagentStop: Enforce bead lifecycle - supervisors must mark inreview
#

INPUT=$(cat)
AGENT_TRANSCRIPT=$(echo "$INPUT" | jq -r '.agent_transcript_path // empty')

[[ -z "$AGENT_TRANSCRIPT" || ! -f "$AGENT_TRANSCRIPT" ]] && echo '{"decision":"approve"}' && exit 0

# Extract last assistant text response using proper JSON parsing
# The old regex approach failed on escaped quotes and special characters
LAST_RESPONSE=$(tail -200 "$AGENT_TRANSCRIPT" | jq -rs '
  [.[] | select(.message?.role == "assistant" and .message?.content != null)
   | .message.content[] | select(.text != null) | .text] | last // ""
' 2>/dev/null || echo "")

# Check for supervisor agents (they must report BEAD_ID and inreview status)
AGENT_TYPE=$(echo "$INPUT" | jq -r '.subagent_type // empty')

if [[ "$AGENT_TYPE" =~ supervisor ]]; then
  # Worker supervisor is exempt from bead requirements (handles small tasks without beads)
  # But still subject to verbosity limits
  IS_WORKER="false"
  if [[ "$AGENT_TYPE" == *"worker"* ]]; then
    IS_WORKER="true"
  fi

  if [[ "$IS_WORKER" == "false" ]]; then
    # Check if this is an epic child task (BEAD_ID contains dot like BD-001.1)
    # Epic children do NOT require code review - review happens at epic level
    BEAD_ID_FROM_RESPONSE=$(echo "$LAST_RESPONSE" | grep -oE "BEAD [A-Za-z0-9._-]+" | head -1 | awk '{print $2}')
    IS_EPIC_CHILD="false"
    if [[ "$BEAD_ID_FROM_RESPONSE" == *"."* ]]; then
      IS_EPIC_CHILD="true"
    fi

    # Supervisors must include completion report
    HAS_BEAD_COMPLETE=$(echo "$LAST_RESPONSE" | grep -cE "BEAD.*COMPLETE" 2>/dev/null || true)
    # Check for EITHER Branch: or Worktree: in completion report
    HAS_WORKTREE_OR_BRANCH=$(echo "$LAST_RESPONSE" | grep -cE "(Worktree:|Branch:).*bd-" 2>/dev/null || true)
    [[ -z "$HAS_BEAD_COMPLETE" ]] && HAS_BEAD_COMPLETE=0
    [[ -z "$HAS_WORKTREE_OR_BRANCH" ]] && HAS_WORKTREE_OR_BRANCH=0

    # Check completion format first (required for all)
    if [[ "$HAS_BEAD_COMPLETE" -lt 1 ]] || [[ "$HAS_WORKTREE_OR_BRANCH" -lt 1 ]]; then
      if [[ "$IS_EPIC_CHILD" == "true" ]]; then
        cat << 'EOF'
{"decision":"block","reason":"Epic child task completion format required:\n\nBEAD {BEAD_ID} COMPLETE\nWorktree: .worktrees/bd-{BEAD_ID}\nFiles: [list]\nSummary: [1 sentence]\n\nRun: bd update {BEAD_ID} --status done\n\nNote: Code review happens at EPIC level after all children complete."}
EOF
      else
        cat << 'EOF'
{"decision":"block","reason":"Supervisor must use completion report format:\n\nBEAD {BEAD_ID} COMPLETE\nWorktree: .worktrees/bd-{BEAD_ID}\nFiles: [list]\nTests: pass\nSummary: [1 sentence]\n\nRun bd update {BEAD_ID} --status inreview first."}
EOF
      fi
      exit 0
    fi

    # All tasks require at least 1 comment
    HAS_COMMENT=$(grep -c '"bd comment\|"command":"bd comment' "$AGENT_TRANSCRIPT" 2>/dev/null) || HAS_COMMENT=0
    [[ -z "$HAS_COMMENT" ]] && HAS_COMMENT=0

    if [[ "$HAS_COMMENT" -lt 1 ]]; then
      cat << 'EOF'
{"decision":"block","reason":"Supervisor must leave at least 1 comment on the bead.\n\nRun: bd comment {BEAD_ID} \"Completed: [brief summary of work done]\"\n\nComments provide context for code review and future reference."}
EOF
      exit 0
    fi

    # Git state verification for ALL tasks (including epic children)
    REPO_ROOT=$(cd "$(git rev-parse --git-common-dir)/.." && pwd)
    WORKTREE_PATH="$REPO_ROOT/.worktrees/bd-${BEAD_ID_FROM_RESPONSE}"

    # Worktree MUST exist - block if missing
    if [[ ! -d "$WORKTREE_PATH" ]]; then
      cat << 'EOF'
{"decision":"block","reason":"Worktree not found at expected path. Supervisors must create worktree first.\n\nRun: curl -X POST http://localhost:3008/api/git/worktree -H 'Content-Type: application/json' -d '{\"repo_path\": \"<REPO>\", \"bead_id\": \"<BEAD_ID>\"}'"}
EOF
      exit 0
    fi

    # Worktree exists - verify git state
    if [[ -d "$WORKTREE_PATH" ]]; then
      # Check 1: Uncommitted changes
      UNCOMMITTED=$(git -C "$WORKTREE_PATH" status --porcelain 2>/dev/null)
      if [[ -n "$UNCOMMITTED" ]]; then
        cat << 'EOF'
{"decision":"block","reason":"Worktree has uncommitted changes.\n\nRun in worktree:\n  git add -A && git commit -m \"...\"\n\nThen report completion."}
EOF
        exit 0
      fi

      # Check 2: Remote push (only if remote exists)
      HAS_REMOTE=$(git -C "$WORKTREE_PATH" remote get-url origin 2>/dev/null)
      if [[ -n "$HAS_REMOTE" ]]; then
        BRANCH="bd-${BEAD_ID_FROM_RESPONSE}"
        REMOTE_EXISTS=$(git -C "$WORKTREE_PATH" ls-remote --heads origin "$BRANCH" 2>/dev/null)
        if [[ -z "$REMOTE_EXISTS" ]]; then
          cat << 'EOF'
{"decision":"block","reason":"Branch not pushed to remote.\n\nRun in worktree:\n  git push -u origin bd-{BEAD_ID}\n\nThen report completion."}
EOF
          exit 0
        fi
      fi

      # Check 3: Bead status (different expected status for epic children vs standalone)
      BEAD_STATUS=$(bd show "$BEAD_ID_FROM_RESPONSE" --json 2>/dev/null | jq -r '.[0].status // "unknown"')
      EXPECTED_STATUS="inreview"
      if [[ "$IS_EPIC_CHILD" == "true" ]]; then
        EXPECTED_STATUS="done"
      fi
      if [[ "$BEAD_STATUS" != "$EXPECTED_STATUS" ]]; then
        cat << EOF
{"decision":"block","reason":"Bead status is '${BEAD_STATUS}', not '${EXPECTED_STATUS}'.\n\nRun: bd update ${BEAD_ID_FROM_RESPONSE} --status ${EXPECTED_STATUS}\n\nThen report completion."}
EOF
        exit 0
      fi
    fi
  fi

  # Enforce concise responses for ALL supervisors (including worker)
  # Note: JSON escapes \n as literal chars, use printf to interpret
  DECODED_RESPONSE=$(printf '%b' "$LAST_RESPONSE")
  LINE_COUNT=$(echo "$DECODED_RESPONSE" | wc -l | tr -d ' ')
  CHAR_COUNT=${#DECODED_RESPONSE}

  if [[ "$LINE_COUNT" -gt 15 ]] || [[ "$CHAR_COUNT" -gt 800 ]]; then
    cat << EOF
{"decision":"block","reason":"Response too verbose (${LINE_COUNT} lines, ${CHAR_COUNT} chars). Max: 15 lines, 800 chars.\n\nUse concise format:\nBEAD {ID} COMPLETE\nWorktree: .worktrees/bd-{ID}\nFiles: [names only]\nTests: pass\nSummary: [1 sentence]"}
EOF
    exit 0
  fi
fi

echo '{"decision":"approve"}'
