---
name: Excession source analysis
about: Run the real Excession CLI on an exact public Git commit through GitHub Actions
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
  "task": "Find the source code for a specific behavior",
  "limit": 16
}
```
