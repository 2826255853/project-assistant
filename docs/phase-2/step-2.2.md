# 步骤 2.2（S2-02）：目录选择与路径授权

状态：有条件通过（Windows 路径授权适配层实现与合成文件系统测试通过，2026-10-26，Asia/Shanghai）。项目仓储、持久化授权和用户可调用项目命令属于步骤 2.3；原生选择器的实际桌面点验尚未完成。阶段 2/M2 未通过。

2026-10-27 接续：2.3 已接入持久仓储、授权/审计事务、启动复核和三个 main-only 命令，见 [步骤 2.3](step-2.3.md)；以下实现与后续说明保留 2.2 当时范围。原生选择器人工点验及 2.5 最终系统交付竞态仍待后续验收。

## 实现范围

`src-tauri/src/project_paths.rs` 暴露 Windows 目录选择、首次授权候选核验和已登记目录重新核验入口；Windows 实现位于 `src-tauri/src/project_paths/windows.rs`。该模块尚未进入 Tauri invoke handler、命令 manifest、前端 `ImplementedCommandMap` 或任何窗口 capability。这样可在 2.3 将核验得到的路径与身份在一个数据库事务中持久化前，避免出现不可恢复的临时项目记录。

- 目录由 Rust 发起 Windows Shell 文件夹选择器，使用本地文件系统目录选择模式。取消表示 `None`；选择器或路径提取失败使用固定 `PA_PROJECT_PICKER_FAILED`。
- 只接受盘符绝对路径；拒绝 UNC/设备命名空间、ADS、`.`/`..`、DOS 设备名、非法字符、尾随点/空格、文件目标和磁盘根目录。只支持本地固定/可移动 NTFS 与 ReFS 卷；网络位置、其他文件系统以及无法读取稳定 File ID 的目标均关闭失败。
- 用 Windows 句柄逐级打开路径组件，并检查每一级 `FILE_ATTRIBUTE_REPARSE_POINT`；任何符号链接、junction 或其他重解析点均拒绝。通过句柄读取每一级规范路径，验证相邻组件仍是同卷的父子路径，并从最终目录句柄取得身份。句柄保留检查到的对象身份，但 Windows 允许并发改名；它本身不锁定路径，因此持久化或实际使用前仍须重新打开路径并比较 File ID。使用扩展 DOS 路径处理超过 `MAX_PATH` 的路径；UI 显示路径不含扩展命名空间前缀。
- 规范路径从最终目录句柄读取；身份由卷序列号和 128-bit 文件 ID 组成，固定 48 位十六进制编码仅供未来项目表存储。调试格式对路径和身份脱敏。
- 路径去重键按 Windows invariant 大小写规则生成；同身份选择返回已有目标，同路径键但身份变化要求显式重新授权，不同路径且不同身份是新目标。路径范围判断按组件比较，避免把字符串前缀当作父子关系。
- 重启/使用前通过 `revalidate_registered_directory` 重新打开目录并比较稳定身份。缺失返回固定路径不可用，访问拒绝返回固定权限错误，身份变化返回 `PA_PROJECT_PATH_CHANGED`；不会自动绑定替代目录。

该适配器提供的是服务端验证与短期身份句柄，不代替 2.3 持久仓储和启动复核，也不代替 2.5 系统打开时的最终目标交付核验。仅检查路径后再交给基于路径的系统打开器无法消除其检查/使用竞态；系统打开命令仍未注册，2.5 必须证明交付目标仍是授权身份，否则拒绝开放该命令。

## 验证

Windows x64 实机 Rust 文件系统测试使用应用自建临时目录，不读取或修改用户项目。`cargo test --locked --manifest-path src-tauri/Cargo.toml --lib project_paths::` 覆盖 11 项：中文/空格/`&`、盘符大小写别名、身份编码、重复选择/显式重新授权决策、路径组件与句柄父子关系、缺失/非目录/UNC/设备/ADS/穿越/DOS 名/尾随点、目录替换身份变化、junction 拒绝、固定错误映射和敏感调试输出脱敏。目录替换测试确认 Windows 可在身份句柄仍打开时改名；同一路径放入新目录后，File ID 复核返回路径变化错误。权限错误通过适配层注入 Win32 access-denied 错误码验证映射；本轮没有改动真实目录 ACL。

测试命令：

```powershell
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
cargo check --locked --manifest-path src-tauri/Cargo.toml
npm run build
npm run test:ipc
```

自动化未操作原生 Shell 选择器的鼠标/键盘流程；首次选择、取消、中文/特殊字符路径在实际对话框中的人工点验仍需后续桌面验收。当前 IPC 项目命令未开放，overlay 和远端仍无路径选择权限。

## 后续

1. 2.3 追加迁移并为路径、去重键及稳定身份建立类型化项目仓储；事务提交前持有 `VerifiedDirectory`，审计失败回滚，启动时重新核验身份。
2. 在项目登记 IPC 接入前，增加取消零写、别名重复登记和并发/数据库失败回滚测试。
3. 2.5 在系统打开前保持身份核验与实际交付目标一致；达不到句柄或等效竞态约束时不开放打开命令。
4. 完成桌面选择器人工点验后更新本记录和 S2-09 验收证据；R-019 的项目路径/系统打开子风险在相关接口通过前保持开启。
