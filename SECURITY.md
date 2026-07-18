# Security Policy

## Supported maintenance scope

This community-maintained fork accepts security reports for issues that can be reproduced against code in this repository.

## How to report a vulnerability

Please avoid posting exploit details in a public issue.

Preferred order:

1. Use GitHub's private vulnerability reporting for this repository if it is enabled.
2. If private reporting is not yet available, open a minimal public issue **without** exploit details and request a private contact path from the fork maintainer.

When reporting, include:

- affected version or commit
- impacted platform
- concise impact summary
- reproduction conditions
- proof-of-concept details only through a private channel

## Response goals

As a community fork, response times are best-effort. The intent is to:

- acknowledge triage quickly
- confirm severity and scope
- prepare a fix or mitigation
- publish a release note once users can take action

## Operational note for this fork

Before shipping official binaries, maintainers should review inherited infrastructure such as:

- update endpoints
- signing keys
- website-hosted APIs
- telemetry or announcement endpoints

Security ownership is incomplete until those dependencies are under the fork maintainer's control.
