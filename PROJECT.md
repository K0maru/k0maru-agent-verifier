# K0maru-Agent-Verifier: 确定性质量门禁与执行收据签署中枢

## 🎯 项目愿景与使命
作为 **K0maru-Harness** 独立模块化矩阵中的第四块核心积木，构建极简、轻量、客观事实驱动的智能体确定性质量门禁。
坚决贯彻“不信 Agent 口头汇报，只认物理执行事实”，在代码进入 `code-review` 之前，通过对真实进程退出码（Exit Code）、编译器诊断、测试用例跑批与变更范围白名单的物理硬拦截，签发带 SHA-256 校验的不可变执行收据（`ExecutionReceipt`），彻底终结智能体“虚假修复”与“跳步交付”。

## 🧩 核心功能规约
1. **多语言测试与构建运行器（Runner SPI）**：自动检测 Rust (`cargo test && cargo clippy`)、Node (`npm test`)、Python (`pytest`) 等工程栈，捕获标准进程退出码（Exit Code）。
2. **变更范围防腐门禁（Scope Guard）**：对比工单声明的 `allowed_files` / 白名单与实际 `git diff`，杜绝越权破坏无关文件。
3. **不可变防伪收据（`agent-receipt-v1.json`）**：计算 Git Patch SHA-256 指纹、覆盖率快照与测试耗时，持久化收据供 Router / 审查下游查验。
4. **与 Workspace 和 Code-Review 无缝集成**：为 `k0maru-agent-workspace` 的自动合并与 `code-review` 提供前置客观事实凭据。

## 🧭 研发任务清单 (Phase 1)
- [ ] 1. 落地 `src/error.rs` 与 `src/receipt.rs`（标准 `agent-receipt-v1.json` 契约定义与 SHA-256 计算）；
- [ ] 2. 落地 `src/runner.rs`（多语言物理测试运行器，真实捕获 Exit Code 与 stderr 截断）；
- [ ] 3. 落地 `src/guard.rs`（Git Diff 范围白名单校验器）；
- [ ] 4. 落地 `src/main.rs` CLI 子命令：`k0maru-verifier verify --workspace <path> [--spec <spec.json>]`；
- [ ] 5. 编写 100% 覆盖率的自动化回归测试套件。
