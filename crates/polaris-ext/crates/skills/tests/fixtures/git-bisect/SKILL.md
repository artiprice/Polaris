---
name: git-bisect
description: Find the commit that introduced a regression by narrowing a known good and bad range.
requires_tools:
  - git_read
---
# Git bisect

Use `git bisect start`, mark the known bad revision and known good revision, then test the
midpoint selected by Git. Repeat until Git identifies the first bad commit. Always record the
range and test command before beginning, and finish with `git bisect reset`.
