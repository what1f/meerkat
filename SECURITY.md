# Security Policy

## Supported versions

Until the first stable release, security fixes are made on the latest `main`
revision only.

## Reporting a vulnerability

Please use the repository's private Security Advisory flow. Do not open a public
issue containing credentials, tokens, private account identifiers, logs with
sensitive data, or reproducible exploit details.

Include the affected revision, impact, reproduction steps, and any suggested
mitigation. You should receive an acknowledgement within seven days.

## Credential handling

Lankide credentials belong in the product's private storage or in explicitly
configured local test fixtures. They must never be committed to this repository.
If a credential is committed, revoke it immediately before attempting history
cleanup.
