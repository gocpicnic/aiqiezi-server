#!/usr/bin/env bash
#
# scripts/configure-branch-protection.sh
# Copyright 2026 aiqiezi. All rights reserved.
# Use of this source code is governed by the AGPL-3.0
# license that can be found in the LICENSE file.
#
# Configure branch protection for main using the GitHub CLI.
# Requires:
#   - gh installed and authenticated as a repository administrator
#   - jq is not required

set -euo pipefail

REPO="${1:-gocpicnic/aiqiezi-server}"
BRANCH="${2:-main}"

if ! command -v gh >/dev/null 2>&1; then
    echo "error: gh is not installed" >&2
    exit 1
fi

gh api \
    --method PUT \
    "repos/${REPO}/branches/${BRANCH}/protection" \
    --input - <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "contexts": ["fmt", "clippy", "test"]
  },
  "enforce_admins": false,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": false,
    "required_approving_review_count": 0
  },
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "required_conversation_resolution": true
}
JSON

echo "branch protection configured for ${REPO}:${BRANCH}"
