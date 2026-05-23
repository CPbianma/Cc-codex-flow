# 证据归档

本目录收录本轮任务跑出来的运行证据：headless E2E 日志、code-review 输出、流式 tail 演示。

| 文件 | 说明 |
| --- | --- |
| `e2e_dev_natural.log` | `cargo run --example e2e_dev` 自然产出的最新一次完整运行日志 |
| `e2e_dev_full.log` | dev profile 三轮 FSM 全量日志（含 1357B 流式输出） |
| `e2e_visual_full.log` | visual profile 三轮 role-swap FSM 全量日志 |
| `demo_evidence.log` | 综合 demo 跑出的多场景证据（任务创建、干预、状态切换） |
| `finalize_evidence.log` | `reset_task` + 收尾流程证据 |
| `code_review_findings.json` | `/code-review` 在 FSM/commands 上的结构化结论 |
| `streaming_tail_evidence.html` | [L] 流式 tail 渲染演示（静态 HTML） |
| `streaming_tail_evidence.png` | [L] 流式 tail UI 截图证据 |
