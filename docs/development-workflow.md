# FlexiSuite Development Workflow

> **Purpose**: Defines how autonomous development agents (Secretary, Jules, CodeRabbit, Devin, Codex, Gemini) collaborate to deliver production-quality changes — from issue selection through merge and conflict resolution.

---

## 1. Issue Selection & Launch

1. **Select or create an issue.** Decide what to work on next.
2. **Parallelize when possible.** Running multiple issues concurrently is the default.
3. **Validate the issue content.** Ensure the issue body contains enough context for an agent that starts with zero prior knowledge. The issue itself is the primary information source.
4. **Launch Jules.** Pass either the issue URL or the full issue text. Confirm Jules has everything it needs before starting.

---

## 2. Jules: Implementation & PR Creation

- Jules runs in a Google cloud sandbox. It works on a git worktree-like branch and automatically creates a PR upon completion.
- **Session quota**: Up to 100 sessions/day on the paid tier. Exhaustion is extremely unlikely in practice.
- **One new PR = one Jules session.**
- Jules auto-detects CI failures and begins fixing them without prompting.
- Jules may occasionally pause and ask questions. Detect these and relay them to the human promptly.

---

## 3. Review Loop: CodeRabbit + Devin

After Jules creates a PR:

1. **CodeRabbit** and **Devin Review** automatically start reviewing (unless rate-limited).
2. They produce:
   - **PR summaries**
   - **Review comments** with suggested fixes
   - **Nitpicks** (CodeRabbit-specific)

### Nitpick Policy (MDP Compliance)

Per the MDP philosophy, **every nitpick must be evaluated**. Options:

- Fix the nitpick
- Create a new issue for deferred work
- Initiate a constitutional amendment discussion

Ignoring nitpicks entirely is not acceptable.

### Follow-up with Jules

- CodeRabbit typically provides agent-ready prompts in its review.
- Append FlexiSuite-specific guidance to these prompts (e.g., "Do not blindly trust review output; always verify against the constitutional documents").
- Send the combined instruction as a follow-up message to the Jules session that created the PR.

### Devin Flags

Devin does not surface flags in PR comments. Use the Devin bridge to fetch flags from the Devin Review PR page. Evaluate all flags the same way as CodeRabbit nitpicks.

### Iteration

Push → Review → Push cycles continue until reviewers reach consensus that no further changes are needed.

---

## 4. Additional Review: Gemini & Codex (Local)

When CodeRabbit and Devin have no further comments:

1. Request reviews from **local Gemini** and **local Codex**.
2. Reviews are never blindly trusted. Always cross-validate with multiple agents.
3. If fixes are needed, implement and push. CodeRabbit and Devin will review again.

---

## 5. Final Gate: Codex Cloud

When all other reviewers are satisfied:

1. Post `@codex review` as a PR comment to invoke **Codex Cloud** review in its cloud sandbox.
2. If Codex Cloud finds nothing to flag → the PR is ready for merge (CI must also be green).

---

## 6. Merge

- Merge is blocked until:
  - All review rounds are complete (CodeRabbit, Devin, local Gemini, local Codex, Codex Cloud)
  - CI is green
  - No unresolved review signals remain

---

## 7. Post-Merge: Conflict Resolution

1. After merge, check other open PRs for merge conflicts.
2. **Do not use Jules for conflict resolution** — it struggles with conflicts.
3. Use a local agent (subagent, Codex, or Gemini) instead.
4. **Always provide full context** to the resolving agent. An agent without context will produce incorrect merges.

---

## 8. Operational Principles

### Constitutional Compliance

All agents must read and follow the constitutional documents before starting work:

1. `docs/implementation_plan.md` (SSOT)
2. `docs/flexisuite-concept.md` (vision)
3. `docs/negative-space-spec.md` (prohibited actions)
4. `docs/verification_matrix.md` (verification gates)

### Multi-Agent Deliberation

No single agent makes final decisions alone. Use the Audit agent or multi-agent discussion for:
- Architectural choices
- Security-sensitive changes
- Constitutional interpretation questions

When in doubt, return to the MDP perspective: *"What is the right thing to do for a Minimal Desirable Product?"*

---

## 9. Rate Limit & Credit Management

### CodeRabbit

- CodeRabbit maintains a **status message** as the top comment on each PR, continuously edited with:
  - Current review state ("reviewing", "complete", etc.)
  - Rate limit info ("next review available in X minutes")
  - Activity pauses ("this PR has frequent pushes, pausing review")
- **Check the comment's edit timestamp** — the message is not real-time updated.
- To resume a paused review: post `@coderabbitai resume` as a PR comment.
- To re-request review after rate limit clears: wait the indicated minutes, then post `@coderabbitai review`.
- Avoid wasteful review requests — coordinate push timing with rate limit windows.

### Other Providers

Monitor credit balances and rate limits for all agents. Use credits efficiently and maximally, but never waste them.

---

## 10. Agent Roles Summary

| Role | Agent | Scope |
|---|---|---|
| Orchestration & Decision | Secretary (this agent) | Issue selection, review coordination, merge decisions |
| Implementation | Jules | Cloud sandbox, auto PR creation, CI fix |
| External Review | CodeRabbit | PR review, nitpicks, agent prompts |
| External Review | Devin | PR review, flags (via bridge) |
| Local Review | Gemini | Additional cross-validation |
| Local Review | Codex | Additional cross-validation |
| Final Gate Review | Codex Cloud | Cloud sandbox final review |
| Safety Audit | Audit agent | Plan review, risky action preflight |
| Ergonomics | AX Steward | Settings, harness friction resolution |
