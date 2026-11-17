---
name: Excession evidence for an investigation
about: Request source-bound locate/context/verify evidence for a codebase investigation
title: "Excession analysis request"
labels: []
assignees: []
---

```json
{
  "schema": "excession.github-analysis-request.v1",
  "repository": "laurajoyhutchins/excession",
  "revision": "REPLACE_WITH_FULL_40_CHARACTER_LOWERCASE_COMMIT_SHA",
  "operation": "locate",
  "task": "Locate evidence needed to explain a specific behavior",
  "limit": 16
}
```

This request executes **existing** source-evidence operations (`locate`,
`context`, or `verify`), not a complete generated investigation or an
independent behavioral proof. See
[the investigation workflow](../../docs/CODEBASE_UNDERSTANDING.md).
