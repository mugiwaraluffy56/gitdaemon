#!/usr/bin/env bash
# on_push_success hook — posts a Slack message when gd pushes commits.
#
# Prerequisites:
#   1. Create a Slack incoming webhook: https://api.slack.com/messaging/webhooks
#   2. Set SLACK_WEBHOOK_URL in your environment or .env file
#
# Use in gd.yml:
#   hooks:
#     on_push_success: "bash examples/hooks/on-push-slack.sh"
#
# Environment variables provided by gd:
#   $FG_BRANCH   — branch that was pushed
#   $FG_COMMITS  — number of commits pushed

set -euo pipefail

if [[ -z "${SLACK_WEBHOOK_URL:-}" ]]; then
    echo "[on-push-slack] SLACK_WEBHOOK_URL not set — skipping notification"
    exit 0
fi

REPO_NAME=$(basename "$(git rev-parse --show-toplevel)")
LATEST_MSG=$(git log -1 --pretty="%s")

PAYLOAD=$(cat <<EOF
{
  "text": "*gd pushed* to \`${FG_BRANCH}\` in *${REPO_NAME}*",
  "attachments": [
    {
      "color": "#36a64f",
      "fields": [
        { "title": "Commits", "value": "${FG_COMMITS}", "short": true },
        { "title": "Latest", "value": "${LATEST_MSG}", "short": false }
      ]
    }
  ]
}
EOF
)

curl -s -X POST "$SLACK_WEBHOOK_URL" \
    -H 'Content-type: application/json' \
    --data "$PAYLOAD" \
    --fail \
    --silent \
    --show-error

exit 0
