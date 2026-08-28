# Project rules

## Git workflow

不要在 `local_dev` 上直接改代码。新特性或 bug 修复必须先基于 `local_dev` 开分支，在分支上编码、测试，通过后再合回 `local_dev`。

1. 从最新的 `local_dev` 创建分支，例如 `git checkout -b feat/<name>` 或 `fix/<name>`
2. 只在该分支上改 tracked 文件、跑测试
3. 测试通过后再把分支合回 `local_dev`（fast-forward 或 merge 均可）

当前 HEAD 若已是 `local_dev`，开始实现前必须先切到新分支，不要把 `local_dev` 当工作分支。合回 `local_dev` 本身允许，那是集成步骤，不是在上面直接开发。
