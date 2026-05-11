# AI Integration

读者对象：准备接入真实 AI agent 或自动化 runner 的开发者。

本文覆盖范围：当前 AI boundary、快照、digest、人类控制对 AI 的约束。

## Current Boundary

`src/ai.rs` defines:

- `AiCommand`
- `AiEvent`
- `AiNexus`
- `AiNexusHandle`
- `CompositorSnapshot`
- `AiWindowDigest`

This is a local channel boundary. It does not run a model and does not expose network access.

## Command Stream

Current AI commands include:

- request snapshot;
- set clear color;
- focus first window;
- shutdown.

Runtime drains AI commands after human control commands. If human control has paused AI, runtime
rejects AI commands with an action result error.

## Observation Stream

The compositor can emit:

- snapshot;
- action result;
- prompt preview.

The current window digest is mirrored to:

```text
target/ai-input-preview.txt
```

The digest includes active workspace window information and excludes the human control surface.

## Human Override

Human control can:

- pause AI;
- resume AI;
- mark tasks cancelled;
- focus windows;
- switch workspaces;
- inject text;
- shutdown compositor.

A future AI worker must check the paused/cancelled state before applying actions.

## Future Work

- Replace MVP channel-only boundary with a real agent process.
- Add explicit command authorization and source labels.
- Add richer state snapshots.
- Add structured output for scripting and AI tools.
- Add task lifecycle: queued, running, cancelled, failed, completed.
