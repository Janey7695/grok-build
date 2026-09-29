# Project rules

## Git workflow

不要在 `local_dev` 上直接改代码。新特性或 bug 修复必须先基于 `local_dev` 开分支，在分支上编码、测试，通过后再合回 `local_dev`。

1. 从最新的 `local_dev` 创建分支，例如 `git checkout -b feat/<name>` 或 `fix/<name>`
2. 只在该分支上改 tracked 文件、跑测试
3. 测试通过后再把分支合回 `local_dev`（fast-forward 或 merge 均可）

当前 HEAD 若已是 `local_dev`，开始实现前必须先切到新分支，不要把 `local_dev` 当工作分支。合回 `local_dev` 本身允许，那是集成步骤，不是在上面直接开发。

## 编译 release

只编 pager 这一个包，不要编整个 workspace：

```sh
cargo build --release -p xai-grok-pager-bin -j "$(sysctl -n hw.logicalcpu)"
```

1. 默认产物 `target/release/xai-grok-pager`，就是 kk 运行的那个文件
2. 要极限优化时用 `cargo build --profile release-opt -p xai-grok-pager-bin`，产物在 `target/release-opt/xai-grok-pager`。该 profile 是 fat LTO 加单 codegen unit 加去符号，实测链接 19 分钟、114.7MB，对比 `--release` 的 180.4MB
3. 合代码的工作流先编到 `target/wf-release` 做 staging，避免覆盖正在运行的 kk
4. 单条命令超时给到 3600000ms。被 OOM 或信号 9 打断时，用小一点的 `-j` 重试一次

### 磁盘

`target/` 容易涨到百 G 级，本机见过 178G（其中 `target/debug/incremental` 占 88G，是每次改代码累积的增量缓存）。空间不够时按这个顺序清：

1. `rm -rf target/debug/incremental`，纯缓存，Cargo 会自动重建，只损失增量编译加速
2. `cargo clean --release -p xai-grok-pager-bin`，只清这个包的 release 产物
3. `target/wf-debug`、`target/wf-release` 是工作流 staging 目录，不需要时可以删

磁盘写满时 cargo 会报 Finished，却只写出几 KB 的截断二进制，而且 fingerprint 认为它是最新的。这种情况下直接重跑 cargo 不会重编，必须先删掉那个产物，再 `cargo clean --release -p xai-grok-pager-bin` 才会重新链接。

## 签名与安装

macOS 上用 adhoc 签名，identifier 固定 `xai-grok-pager`，不要 strip，先签 staging 文件：

```sh
codesign -s - --force --identifier xai-grok-pager <staging>
codesign --verify --verbose <staging>
```

装到 kk 指向的路径时用原子替换，不要删除或覆盖正在运行的文件：

```sh
cp <staging> target/release/xai-grok-pager.new
codesign -s - --force --identifier xai-grok-pager target/release/xai-grok-pager.new
codesign --verify target/release/xai-grok-pager.new
mv -f target/release/xai-grok-pager.new target/release/xai-grok-pager
```

1. `cp` 会让 Mach-O 签名失效，所以拷成 `.new` 之后必须重新签名；`.new` 上有 `com.apple.quarantine` 就用 `xattr -d` 删掉
2. `mv -f` 只替换目录项，正在运行的 kk 会话继续用旧 inode，不要 kill 它。替换前不要 `rm` live 路径，也不要 cp 或 truncate 到它上面
3. 装完跑一次绝对路径的 `--version`，退出 0 且打印 grok 版本行才算成功。退出 137 或被 Killed 时，对 live 路径重签一次再试

## kk 与全局安装

kk 是 zsh 别名，定义在 `~/.oh-my-zsh/custom/aliases.zsh`，指向 `/Users/bytedance/Public/grok-build/target/release/xai-grok-pager`。验证本地构建一律用 `kk`，或直接跑这个绝对路径。

`~/.grok/bin/grok` 是官方发布版的符号链接，指向 `~/.grok/downloads/grok-<version>-macos-aarch64`，`~/.grok/bin/agent` 指向同一个文件。这两个路径属于全局安装，不要为了测试本地构建去改它们，本地构建也用不着它们。
