# Security policy

## Supported versions

`git-why` has not published a release yet. Security fixes are applied to the current main branch. This policy will be updated when supported release lines exist.

## Reporting a vulnerability

Please do not disclose a suspected vulnerability in a public issue, discussion, pull request, or commit.

Use GitHub's private vulnerability reporting option in the repository's Security tab when it is available. If it is not available, contact a maintainer privately using the contact information on their GitHub profile and ask for a secure reporting channel before sharing sensitive details.

Include, where possible:

- the affected revision and environment;
- a minimal reproduction;
- the security impact and required attacker access;
- whether the issue is already public or has a disclosure deadline;
- any suggested mitigation.

Do not include real credentials, private repository content, or personal data in a reproduction. The maintainers will coordinate validation, remediation, and disclosure with the reporter. No fixed response deadline is promised while the project is pre-release.

## Security expectations

`git-why` reads local repository evidence without invoking a shell, contacting remotes, prompting for credentials, or modifying the repository. Report any behavior that violates these constraints.
