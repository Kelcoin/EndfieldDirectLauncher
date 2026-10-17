# EndfieldDirectLauncher

<img src="assets/icon.png" width="120" align="right" alt="图标">

绕过《明日方舟：终末地》官方启动器，直接启动 `Endfield.exe` 进入游戏。

把 `EndfieldDirectLauncher.exe` 放到游戏目录（与 `Endfield.exe` 同一层）双击即可，
**不需要**先开鹰角启动器，也不会弹出启动器界面。

## 项目简介

游戏本体不看命令行、注册表和资源文件，它看的是**父进程的映像名**。游戏目录里的
`OneChannelLC.dll`（LC = launch check）在 SDK 初始化时读取自身父进程的路径，只接受启动器的
`Games.exe`；父进程是 `explorer.exe`、`steam.exe` 或别的程序，游戏就提示「请用官方启动器启动」
并拒绝进入：

```
[Error] [SDK] [F:3][DLog]PC launch auth extraInfo failed, code: 1, errorCode: 10100, filePath: f:\steam\steam.exe
```

本程序**不修改任何游戏文件**，它做的是改父进程归属：

1. 以 `CREATE_SUSPENDED` 拉起启动器目录里的 `Games.exe`——它存在，但一条指令都不会执行，
   所以启动器界面不会出现，也不会触发更新检查。
2. 用 `InitializeProcThreadAttributeList` + `UpdateProcThreadAttribute` 声明
   `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`，把这份挂起的 `Games.exe` 指定为新进程的父进程。
3. 以该属性 `CreateProcessW` 启动 `Endfield.exe`，内核因此报告父进程是 `games.exe`，检查通过。
4. 游戏退出后终止那份挂起的 `Games.exe`，不留残留进程。

命令行与官方完全一致（取自启动器自己的 `games.log`）：

```
"…\games\Endfield Game\Endfield.exe" -launcher_lang=zh-cn -launcher_sub_channel=1
```

技术形态：纯 Rust，无第三方依赖，全部走 Win32 裸 FFI；`src/main.rs` 一个文件，
`build.rs` 只负责两件构建期资源（管理员权限清单、图标）。

## 如何使用

### 1. 放好文件

把 `EndfieldDirectLauncher.exe` 放到游戏目录，也就是 `Endfield.exe` 那一层：

```
F:\Hypergryph Launcher\games\Endfield Game\
├─ Endfield.exe
├─ EndfieldDirectLauncher.exe   ← 放这里
└─ ...
```

文件从本仓库 Releases 下载，或自行构建（见第 5 节）。

### 2. 双击启动

双击即可，首次会请求管理员权限（原因见 FAQ）。游戏退出后本程序自动退出。

> [!NOTE]
> 游戏内更新仍然要走官方启动器。客户端版本落后时，先正常启动一次官方启动器完成更新，
> 之后继续用本程序。

### 3. 加进 Steam 库（可选）

1. Steam →「添加非 Steam 游戏」→ 选中游戏目录里的 `EndfieldDirectLauncher.exe`。
2. 启动后库条目会一直显示「运行中」，直到游戏退出。
3. 若库里只闪一下就没了，删掉（或改名）游戏目录里的 `steam_appid.txt`，详见 FAQ。

### 4. 命令行参数

```
EndfieldDirectLauncher.exe                      # 游戏目录下双击即可
EndfieldDirectLauncher.exe --console            # 附带控制台窗口，看进度
EndfieldDirectLauncher.exe --game-dir <目录>    # 指定 Endfield.exe 所在目录
EndfieldDirectLauncher.exe --launcher-dir <目录># 指定启动器目录，如 "F:\Hypergryph Launcher"
EndfieldDirectLauncher.exe --help
```

错误一律用消息框弹出。

### 5. 从源码构建

```
cargo build --release
```

产物为 `target\release\EndfieldDirectLauncher.exe`。构建时会调用 Windows SDK 的 `rc.exe`
把 `assets\app.ico` 编成资源嵌进 exe；若 `rc.exe` 不在 PATH 也不在 SDK 默认位置，
设置环境变量 `RC` 指向它即可。

### 6. 出问题看日志

每次运行的完整轨迹（含父进程、PID、启动检查结果）追加写在 exe 同目录的
`EndfieldDirectLauncher.log`。

## 常见问题

### 为什么不能直接双击 `Endfield.exe`？

因为父进程名不是 `Games.exe`。哪怕从 `explorer.exe`、Steam、其他工具启动，游戏都会在 SDK
初始化阶段判定失败并退出。必须让父进程是 `Games.exe`——这正是本程序做的事。

### 为什么需要管理员权限？

`Endfield.exe` 自己的清单里写着 `requestedExecutionLevel level="requireAdministrator"`，
游戏本体永远以高完整性级别运行，普通权限的进程根本创建不了它（`CreateProcessW` 会以
Win32 **740** `ERROR_ELEVATION_REQUIRED` 失败）。本程序同样在清单里声明 `requireAdministrator`，
让 Windows 在启动时提权一次，之后全程只有一个进程：

- Steam 才会一直认为它在运行（进程一退，Steam 就取消「运行中」）；
- 整条进程链都属于同一个进程，行为可预期。

### 会修改游戏文件吗？有封号风险吗？

不修改。程序只创建两个进程，不打开游戏进程句柄、不按进程名扫描（游戏启动期间按进程名检查
有触发 ACE 的风险，因此这里只等待句柄）。唯一需要动到的文件是 Steam 那条里的
`steam_appid.txt`（见下一节），它本来就是游戏自己运行时写的。

绕过官方启动器不是官方支持的行为，服务端是否另有校验没有做过长测，请自行判断风险。

### Steam 库里不显示「运行中」，只闪一下就没了？

游戏目录里有个 `steam_appid.txt`，内容是 `457962`——那是终末地的 Steam AppID，而你的库条目
用的是另一个 id。Steam 启动时读到这个文件，就把进程算到 457962 头上，于是运行列表里永远
没有你这个条目：

```
[2026-10-18 02:24:38] AppID 457962 adding PID 51240 as a tracked process "...\EndfieldDirectLauncher.exe"
[2026-10-18 02:24:57] AppID 457962 no longer tracking PID 51240, exit code 0
```

删掉（或改名成 `steam_appid.txt.bak`）即可，Steam 会改用快捷方式自己的 id。
注意这个文件会被游戏在每次启动时按当前会话重写，所以它可能自己又长回来。

### Steam 覆盖层（Shift+Tab）和 F12 截图为什么用不了？

> [!WARNING]
> 在游戏要求管理员权限的前提下，Steam 覆盖层**无法**工作，这不是本程序可以绕过的。

Steam 启动游戏时会给它一个降权的令牌，因此本程序必须自行提权，于是「Steam 注入覆盖层的那个
进程」和「真正去启动游戏的提权进程」不是同一个；而非提权进程也无法把 DLL 注入到高完整性级别
的 `Endfield.exe` 里。Steam 自己的覆盖层日志也只记录了提权之前那一次注入：

```
Sat Oct 17 17:59:44 2026 UTC - Current process: EndfieldDirectLauncher.exe
GameID = 457962, OverlayGameID = 13909082581166456832
Sat Oct 17 17:59:50 2026 UTC - GameOverlayRenderer.dll detaching
```

替代方案：NVIDIA 覆盖层（`Alt+Z` 打开、`Alt+F1` 截图），或用游戏自带的截图键。

### 支持国际服 / B 服 / Steam 版吗？

未验证。代码里有目录兜底，理论上同样可用；据社区反馈国际服（GRYPHLINK）本来就不做这项检查。
本机只验证过国服 1.6.0 客户端。

### 游戏更新怎么办？

绕过启动器意味着不走更新流程。发现版本落后时，用官方启动器正常启动一次完成更新，
之后继续用本程序。

### 启动器目录是怎么找到的？

按 MaaEnd 的顺序查找：卸载表 `InstallLocation`（国服 key
`33a0a6296a20400d503c59ac0fd6341e`，含 `WOW6432Node` 视图）→ URL 协议
`hypergryphlauncher-abYeZZ16BPluCFyT` 的命令行 → 兜底 `<游戏目录>\..\..`。
`Games.exe` 取启动器根目录或版本子目录（如 `1.6.0\Games.exe`）中最新的一个。

### 图标是哪来的？

从 `Endfield.exe` 里提取的：`assets/extract-icon.py` 解析 PE 资源目录的
`RT_GROUP_ICON` / `RT_ICON`，还原出 9 档尺寸（256² 到 16²）的 `assets/app.ico`，
构建时由 `build.rs` 经 `rc.exe` 嵌进 exe。图标版权归游戏方所有。

### 验证到什么程度？

同一台机器、同一个游戏版本，只换启动方式做对照：

| | 直接启动 `Endfield.exe` | 本程序 |
| --- | --- | --- |
| `PC launch auth failed / errorCode 10100` | 出现 | 整份 `Player.log` 里一条都没有 |
| 游戏窗口 | 无，提示「请用官方启动器启动」 | `Endfield`，`Responding = True` |
| 进入渲染 | 否 | 是（`HGCreateSwapchain`） |
| `Endfield.exe` 的父进程 | `explorer.exe` / `steam.exe` | 挂起的 `Games.exe` |

**未验证**：国际服 / B 服 / Steam 版；服务端是否另有校验（没有做长时间游玩测试）；
游戏内更新路径。
