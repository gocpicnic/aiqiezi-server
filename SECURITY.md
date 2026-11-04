<!--
  SECURITY.md
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
-->

# Security Policy

## Supported versions

The project is in early development. Security fixes are applied to the current
`main` branch and the latest release, if a release exists.

## Reporting a vulnerability

Please do not open a public GitHub issue for security vulnerabilities.

Use GitHub's private vulnerability reporting:

<https://github.com/gocpicnic/aiqiezi-server/security/advisories/new>

Include:

- affected version or commit;
- a description of the issue;
- reproduction steps or a proof of concept;
- impact assessment;
- any suggested fix.

## Scope

Security-sensitive areas include:

- iroh EndpointId whitelist enforcement;
- public iroh JSON and iroh-blobs handlers;
- local admin Unix socket permissions;
- PostgreSQL access and migrations;
- media storage path handling and hash verification;
- secrets and configuration handling.

## Disclosure

Please allow time for triage and a fix before public disclosure. Confirmed
vulnerabilities may be disclosed through a GitHub Security Advisory.
