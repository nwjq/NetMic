---
name: netmic-harness-git-sync
description: 在 NetMic 双机 Harness 中，当本地 macOS 仓库和 Linux 远端仓库都指向同一个 Git origin 时，使用这套流程核查是否适合走 git push/pull，同步前识别 remote dirty、branch 偏移和 HEAD 不一致，并给出安全结论。适用于排查“为什么远端代码和本地不一致”“是否可以改成 push/pull 而不是 rsync”这类问题。
---

# NetMic Harness Git Sync

只在以下条件同时成立时使用：

- 当前问题是 NetMic 双机 Harness 的代码同步
- 本地仓库和 Linux 远端仓库都存在 `.git`
- 用户希望确认能否改用 `git push/pull`

## 快速流程

1. 读取本地 `.harness/hosts.env`
   - 关注 `NETMIC_HARNESS_LINUX_HOST`、`NETMIC_HARNESS_LINUX_USER`、`NETMIC_HARNESS_LINUX_ROOT`
2. 核对本地与远端仓库的 `origin`
   - 本地：`git remote -v`
   - 远端：`ssh ... 'cd <root> && git remote -v'`
   - 若 `origin` 不一致，直接判定不能走同源 `push/pull`
3. 核对本地状态
   - `git branch --show-current`
   - `git rev-parse HEAD`
   - `git status --short --branch`
   - 对 NetMic Harness，优先参考 runner 的 repo snapshot 口径：`.harness/hosts.env`、`.harness/runs/`、`.codex/`、`AGENTS.md` 不应影响“产品代码是否一致”的判断
4. 核对远端状态
   - `ssh ... 'cd <root> && git branch --show-current && git rev-parse HEAD && git status --short --branch'`
   - 若远端 dirty，不要直接建议 `git pull`
5. 给出结论
   - 本地 clean + 远端 clean + 同 branch + 同 origin：可走 `git push` 后 `git pull --ff-only`
   - 远端 dirty：标记为风险，先让用户决定是保留、stash 还是清理；不要擅自 `reset --hard`
   - 本地有未提交产品代码：优先提交，再谈 `push/pull`

## 结论模板

- `可改 git sync`
  - 条件：local clean、remote clean、同 origin、同 branch
  - 动作：本地 `git push origin <branch>`，远端 `git pull --ff-only origin <branch>`
- `暂不适合 git sync`
  - 条件：remote dirty / branch 不一致 / origin 不一致
  - 必须指出具体阻塞项，不要只说“不同步”

## 安全边界

- 不要在未获明确同意时对远端执行 `git reset --hard`、`git clean -fd`
- 若远端 dirty，大概率说明它已经被 `rsync` 或手工改动污染；先把变更路径列出来，再决定清理策略
- 若只是为了 Harness 验证，优先让远端仓库恢复到“干净可 pull”状态，而不是继续让两边长期各自漂移
