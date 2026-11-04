<!--
  docs/branch-protection.md
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
-->

# Branch Protection

The repository is public, so `main` should require CI before merge.

## Required settings

For the `main` branch:

- require pull requests before merging;
- require status checks:
  - `fmt`
  - `clippy`
  - `test`
- require branches to be up to date before merging;
- require conversation resolution before merging;
- disallow force pushes;
- disallow branch deletion.

Do not require an approving review for the initial single-owner workflow. Add
that requirement when other contributors join.

## GitHub UI

Open:

```text
Settings -> Branches -> Add branch protection rule
```

Set the branch pattern to `main`, then enable the settings above.

## GitHub CLI

If `gh` is installed and authenticated as a repository administrator, use the
helper script:

```bash
./scripts/configure-branch-protection.sh gocpicnic/aiqiezi-server
```

This script uses the GitHub API. It is intentionally not run by CI.

## Security considerations

- Keep deployment credentials out of the repository.
- Keep production deployment secrets in GitHub Environments or the server host.
- Do not use `pull_request_target` for untrusted pull requests.
- Review dependency updates before merging.
