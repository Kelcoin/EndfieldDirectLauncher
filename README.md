# EndfieldDirectLauncher

<img src="assets/icon.png" width="120" align="right" alt="图标">

绕过《明日方舟：终末地》官方启动器，直接启动 `Endfield.exe` 进入游戏。
单文件、无第三方依赖、不修改游戏文件。

**平台** Windows 10/11 · **实现** Rust，裸 Win32 FFI · **许可** [MIT](LICENSE)

## 目录

- 功能
- 快速开始
- 命令行参数
- 与自动化脚本一起用（MaaEnd）
- 日志与排错
- 常见问题
- 工作原理
- 已知限制与未验证项
- 验证记录
- 开发
- 许可

## 功能

- **不用装官方启动器**：把 exe 放进游戏目录双击即进游戏，不弹启动器界面。
- **单文件、无依赖**：一个 exe，不需要运行库、不需要安装。
- **不碰游戏本体**：只从一个带发行商签名的 exe 复制出临时副本当父进程，结束后删除。
- **能加进 Steam 库**：库里正常显示「运行中」，退出后自动收尾（含清掉 `steam_appid.txt`）。
- **参数可控**：`--no-reshade` 在装了 ReShade 时关掉后期处理；`--dry-run` 只看不跑，`--no-popup` 给无人值守用。
- **游戏自己的开关能传进去**：本程序不认识的参数原样加在 `Endfield.exe` 命令行末尾，`-force-d3d11`、`-force-vulkan`、`-windowed` 这类渲染后端开关因此可用。

## 快速开始

1. 从 [Releases](../../releases) 下载 `EndfieldDirectLauncher.exe`。
2. 放进游戏目录，也就是 `Endfield.exe` 所在的那一层，不需要装鹰角启动器：

```
F:\Hypergryph Launcher\games\Arknights Endfield\
├─ Endfield.exe
├─ EndfieldDirectLauncher.exe   ← 放这里
└─ ...
```

3. 双击运行，第一次会问管理员权限，点「是」。程序在后台等游戏退出，游戏一关它自己也关。

> 游戏内更新仍要走官方启动器：版本落后时先正常开一次官方启动器更新完，再回来用这个。
> 想加进 Steam 库见下文「常见问题」里的启动方式说明。

## 命令行参数

大多数情况**不用写任何参数**，双击就够了。要加参数，可以接在快捷方式「属性 → 目标」后面，
也可以直接在命令行里写，多个参数可以一起用，例如 `EndfieldDirectLauncher.exe --console --no-reshade`。

### 参数一览

| 参数 | 说明 |
| --- | --- |
| `--console` | 多开一个控制台窗口，逐步显示进度与错误。 |
| `--game-dir <目录>` | 指定 `Endfield.exe` 所在目录，exe 不在游戏目录里时才需要。 |
| `--no-reshade` | 本次启动不加载 ReShade 及其 DLSS 5 插件。装了 ReShade 又要跑画面识别自动化时才加。 |
| `--env NAME=VALUE` | 给游戏进程加一条或改一条环境变量，可重复。 |
| `--dry-run` | 只打印将要执行的命令行与环境变量，不启动游戏。 |
| `--no-popup` | 出错不弹框，只写日志。给脚本与计划任务用。 |
| `--help` | 打印帮助（首行带版本号）。 |
| `<其它参数>` | 本程序不认识的参数原样传给游戏本体，例如 `-force-d3d11`、`-force-vulkan`、`-windowed`。 |

### 逐个说明

**`--console`**

多开一个黑色控制台窗口，本程序每做一步都在里面打一行。想看着它到底干了什么时才加，平时不用。

**`--game-dir <目录>`**

指定 `Endfield.exe` 所在的目录。只有在 exe 不在游戏目录里、或者你想给别的区的客户端启动时才要写。

`--game-dir "D:\game"` 和 `--game-dir="D:\game"` 两种写法都行，目录里有空格就加引号。

**`--no-reshade`**

这一次启动不带 ReShade，装在里面的 DLSS 5 那三个插件也一起不加载。
没装 ReShade 就没东西可关，加不加一样，加了也没副作用。

它只给游戏进程加两条环境变量，不删文件、不改注册表、不动游戏目录，所以下次不带这个参数启动就自动恢复：

- `DISABLE_VK_LAYER_reshade_1=1` —— ReShade 自己声明的开关；
- `VK_LOADER_LAYERS_DISABLE=VK_LAYER_reshade` —— Vulkan loader 按层名过滤，将来 ReShade 改了变量名这条也还管用。

只想关掉 ReShade 里的某几个插件、而保留 ReShade 本身，得去改 `ReShade.ini` 的 `[ADDON]` 段。
那个文件 ReShade 退出时会重写，所以本工具不代写。

**`--env NAME=VALUE`**

给游戏进程加一条或改一条环境变量，可以写多次，例如 `--env FMT=1 --env LC_ALL=zh_CN.UTF-8`。
同样的名字写两次，以最后一次为准。`--env NAME=VALUE` 和 `--env=NAME=VALUE` 都认。

只想临时改而不想留痕迹时用它比改系统环境变量干净：只影响这一次启动的游戏进程。

**`--dry-run`**

只打印准备执行的命令行和环境变量，游戏不启动。排查「参数到底有没有生效」时先跑这个。

**`--no-popup`**

出错也不弹消息框，只写日志。给脚本、计划任务、远程或无人值守启动用——没人去点「确定」时，
弹出的框会把进程一直挂在那儿。

**`--help`**

打印上面这些说明，第一行带版本号，可以用来确认手上这份 exe 是新是旧。`-h` 是同一个参数的短写法。

**`<其它参数>`**

本程序不认识的参数会被原样拼到游戏命令行末尾，排在它自己固定发送的 `-launcher_lang=zh-cn -launcher_sub_channel=1` 后面。
Unity 那套渲染后端开关就是这么用的：

```text
EndfieldDirectLauncher.exe -force-d3d11
EndfieldDirectLauncher.exe -force-vulkan -windowed
```

转发是照抄，不做校验也不改名：写错一个字母，游戏会当成未知开关忽略，不会报错。
上面的 `--xxx` 是本程序自己的参数，不会被转发；`--game-dir`、`--env` 后面跟的值也不会被当成开关。
日志里的 `game args :` 一行记录最终拼出来的命令行，拿不准时先加 `--dry-run` 看它会传什么。

### 通用规则

- 同一个参数写多次，**最后一次生效**；
- `--game-dir` 或 `--env` 后面紧跟另一个 `--参数` 时，会被判成「漏了值」并直接报错，不会把那个参数当成值；
- 出错默认弹消息框（双击启动时这是唯一能看到错误的地方）；从命令行启动或者加了 `--console` 时只写控制台和日志，加了 `--no-popup` 则一律不弹。

## 与自动化脚本一起用（MaaEnd）

**用 MaaEnd 这类靠画面识别的自动化、并且游戏里装了 ReShade 时，`--no-reshade` 才要加。**
滤镜会改色调、对比度和锐度，挂在 ReShade 里的 DLSS 5 插件还会改有效分辨率和细节；截图跟参考图对不上，
脚本就会认错按钮、点空、在同一个界面反复重试，看着像是「脚本坏了」。MaaEnd 里的启动程序 / 自定义启动参数要落到这一条上：

```
EndfieldDirectLauncher.exe --no-reshade
```

只指向官方启动器、或者让脚本绕开本启动器自己去拉游戏，都带不上这个参数。

想确认参数真的送到了，看游戏目录 `EndfieldDirectLauncher.log` 的第二行 `arguments : [...]`。

## 日志与排错

每次运行都会往 exe 同目录的 `EndfieldDirectLauncher.log` 追加记录。开头两行最有用：

```
launcher  : endfield-direct-launcher <版本>
arguments : ["--no-reshade"]
```

- 第一行是版本：确认游戏目录里跑的是不是最新那份 exe；
- 第二行是这次**真正收到**的参数：这一行里没有，就说明是快捷方式、脚本那一层没把参数送进来。

日志超过 1 MiB 会轮转：当前文件改名成 `EndfieldDirectLauncher.log.old`（旧的 `.old` 先删掉），新文件从头写。

确认参数真的送到了，就在第二行找它；有参数却没效果，先比 `launcher` 那一行与这份 exe 的构建是否一致——
更新过仓库却忘了把新 exe 复制进游戏目录时，旧版本会把这个参数当未知参数直接忽略，表现和「参数不管用」一模一样。

## 常见问题

### 怎么让游戏换渲染后端（DX11 / DX12 / Vulkan）？

把 Unity 的开关当普通参数接上即可，例如 `EndfieldDirectLauncher.exe -force-d3d11`：本程序不认识的参数会原样交给 `Endfield.exe`。

但**这个客户端到底编进了哪个后端，本程序管不了**：它只负责把参数送进去，游戏自己不支持就会忽略或起不来。
实测本机默认走的是 Vulkan（`-force-d3d11` 的表现见「已知限制与未验证项」）。

### 为什么不能直接双击 `Endfield.exe`？

因为父进程既不是 `Games.exe`，也拿不出发行商的签名。哪怕从 `explorer.exe`、Steam、其他工具启动，
游戏都会在 SDK 初始化阶段判定失败并退出。必须让父进程是一个**名叫 `Games.exe` 且由发行商签名的映像**
——这正是本程序做的事。

### 为什么需要管理员权限？

`Endfield.exe` 自己的清单里写着 `requestedExecutionLevel level="requireAdministrator"`，
游戏本体永远以高完整性级别运行，普通权限的进程根本创建不了它（`CreateProcessW` 会以 Win32 **740**
`ERROR_ELEVATION_REQUIRED` 失败）。本程序同样在清单里声明 `requireAdministrator`，让 Windows 在启动时
提权一次，之后全程只有一个进程：Steam 才会一直认为它在运行，整条进程链的行为也可预期。

### 会修改游戏文件吗？有封号风险吗？

不改游戏本体。程序只**读出**一个带签名的 exe 复制到 `%TEMP%\EndfieldDirectLauncher\<PID>\`，
不写游戏目录里的任何 exe、dll 或游戏数据；也不打开游戏进程句柄、不按进程名扫描（游戏启动期间按进程名
检查有触发 ACE 的风险，因此这里只等待句柄）。

唯一会被写动的是游戏目录里的 `steam_appid.txt`——程序在游戏退出后把它删掉。这个文件本来就是游戏自己
运行时写的，删它是为了修掉它引起的 Steam 归属错乱（见下文）。

绕过官方启动器不是官方支持的行为，服务端是否另有校验没有做过长测，请自行判断风险。

### 那个当父进程的 `Games.exe` 是从哪来的？

从游戏目录里挑一个带发行商签名的 exe，默认 `PlatformProcess.exe`，依次退到 `QtWebEngineProcess.exe`、
`UnityCrashHandler64.exe`，复制成 `%TEMP%\EndfieldDirectLauncher\<本次运行的 PID>\Games.exe` 再挂起。

检查只认「名字 + 发行商签名」，而游戏目录里的这些文件本来就由该区服的发行商签名（国服是 Shanghai Hypergryph），
所以哪个区服就用哪个区服的签名——不必去找官方启动器目录，也不必猜它的版本子目录。游戏退出后这份副本会被删掉。

### 怎么加进 Steam 库？

1. Steam →「添加非 Steam 游戏」→ 选游戏目录里的 `EndfieldDirectLauncher.exe`；
2. 从库里启动，条目会一直显示「运行中」，直到游戏退出；
3. 要是只闪一下就没了，看下一条 `steam_appid.txt`。

### Steam 库里不显示「运行中」，只闪一下就没了？

游戏目录里的 `steam_appid.txt` 是一个 AppID 提示。Steam 启动进程时会读它：读到 `457962`（终末地的 Steam
AppID，而你的库条目并不是这个 id），就把进程算到 `457962` 头上，于是运行列表里永远没有你这个条目：

```
[2026-10-18 11:20:58] AppID 457962 adding PID 33988 as a tracked process "...\EndfieldDirectLauncher.exe"
[2026-10-18 11:35:59] AppID 457962 no longer tracking PID 33988, exit code 0
```

文件不存在时，Steam 就按快捷方式自己的 id 记：

```
[2026-10-18 02:50:09] AppID 13909082581166456832 adding PID 20860 as a tracked process ...
```

这个文件由游戏运行时写入，写进去的是**那一次会话**的 id。所以只要用过一次官方启动器，或手动双击过本程序，
它就会留下 `457962`，下一次从 Steam 启动就被带偏——这也是为什么它会「自己又长回来」。

本程序会在游戏退出后把它删掉，让下一次 Steam 启动重新回到「没有提示」的正确状态。若你现在正卡在这个状态：
删掉 `steam_appid.txt`，再从 Steam 库启动一次即可。

> 用过官方启动器（例如更新）之后，游戏会再写回 `457962`，紧接着的那一次 Steam 启动可能仍不显示「运行中」；
> 本程序会在退出时把它清掉，所以从那之后（或手动双击启动一次之后）就恢复正常。

## 工作原理

游戏本体不看命令行、注册表和资源文件，它看的是**父进程的映像**。游戏目录里的 `OneChannelLC.dll`
（LC = launch check）在 SDK 初始化时读取自身父进程的路径，然后查两件事：

1. 映像文件名必须是 `Games.exe`（启动器的启动助手）；
2. 该映像必须带**客户端发行商**的有效 Authenticode 签名。

两条都满足才放行，失败码还不一样：名字不对是 `Not found matched name` + `errorCode: 10100`；名字对但没签名是
`Parent process verify signature failed:…, 2148204800`（`0x800B0100` `TRUST_E_NOSIGNATURE`）+ `errorCode: 10101`。
两种情况游戏都会记录 `PC launch auth extraInfo failed` 并提示「请用官方启动器启动」：

```
[Error] [SDK] [F:3][DLog]PC launch auth extraInfo failed, code: 1, errorCode: 10101, filePath: …
```

> 签名校验就在 `OneChannelLC.dll` 里，但那个模块的导入表里**没有** `WinVerifyTrust`——它用 `LoadLibraryExA` +
> `GetProcAddress` 动态取，所以只翻导入表会把它误判成「只比名字」。

本程序不改动游戏本体，它把游戏目录里一个**带发行商签名**的 exe 复制到临时目录，让这份副本充当父进程——
检查只看名字和签名，不看路径，所以副本放临时目录一样通过，**因此完全不需要安装官方启动器**。每次运行各用
一个 PID 子目录，是为了让上一次被强杀留下的副本（Windows 要过一会儿才放开映像节）不会挡住这一次的复制：

1. 复制那份签名的 exe 成临时文件 `Games.exe`，以 `CREATE_SUSPENDED` 拉起——它存在，但一条指令都不会执行；
2. 用 `InitializeProcThreadAttributeList` + `UpdateProcThreadAttribute` 声明 `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`，
   把这份挂起的 `Games.exe` 指定为新进程的父进程；
3. 以该属性 `CreateProcessW` 启动 `Endfield.exe`，内核因此报告父进程是 `games.exe`，两条检查都通过；
4. 游戏退出后终止那份挂起的 `Games.exe`、删掉临时副本，不留残留。

> 那份副本会被放进一个 `KILL_ON_JOB_CLOSE` 的 job 对象，这样本程序被强杀时它也不会残留（挂起的进程永远不会
> 自己退出）。job 是在游戏进程**创建之后**才挂上的：用 `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` 创建的子进程
> 会继承**名义父进程**的 job，挂早了会把游戏也圈进去，那样杀掉本程序就等于杀掉游戏。

固定发送的两条参数与官方启动器一致（取自启动器自己的 `games.log`）；用户额外加的开关会接在它们后面：

```
"…\games\Endfield Game\Endfield.exe" -launcher_lang=zh-cn -launcher_sub_channel=1
```

## 已知限制与未验证项

### Steam 覆盖层（Shift+Tab）和 F12 截图用不了

> **警告**：在游戏要求管理员权限的前提下，Steam 覆盖层**无法**工作，这不是本程序可以绕过的。

Steam 启动游戏时会给它一个降权的令牌，因此本程序必须自行提权，于是「Steam 注入覆盖层的那个进程」和
「真正去启动游戏的提权进程」不是同一个；而非提权进程也无法把 DLL 注入到高完整性级别的 `Endfield.exe` 里。
Steam 自己的覆盖层日志也只记录了提权之前那一次注入：

```
Sat Oct 17 17:59:44 2026 UTC - Current process: EndfieldDirectLauncher.exe
GameID = 457962, OverlayGameID = 13909082581166456832
Sat Oct 17 17:59:50 2026 UTC - GameOverlayRenderer.dll detaching
```

替代方案：NVIDIA 覆盖层（`Alt+Z` 打开、`Alt+F1` 截图），或用游戏自带的截图键。

### 换渲染后端：`-force-d3d11` 本机能传进去，但客户端会自己退出

本程序只负责把开关转交给 `Endfield.exe`，后端支持与否由游戏决定。本机实测（2026-11-27，国服客户端）：

- ReShade 日志里出现 `Redirecting D3D11CreateDevice … FeatureLevels = 1`（feature level 11_0）与
  `IDXGIFactory2::CreateSwapChainForHwnd`，说明参数确实进了游戏、D3D11 设备和交换链都建起来了；
- 但进程在约两分钟后以**退出码 0** 自己结束，窗口始终没出现，Windows 应用日志里也没有崩溃事件
  ——不是崩溃，是客户端主动退出。

结论：这个客户端能跑的是默认后端，`-force-d3d11` 属于「能传进去但跑不起来」。`-force-d3d12` 与
  `-force-vulkan` 未实测。

### 未验证项

- **国际服 / B 服 / Steam 版**：父进程用的是游戏目录自己的签名文件，所以签名天然是那个区服的，理论上同样可用；
  据社区反馈国际服（GRYPHLINK）本来就不做这项检查。本机只验证过国服 1.6.0 客户端。
- **服务端是否另有校验**：没有做长时间游玩测试。
- **游戏内更新**：绕过启动器意味着不走更新流程，需要先用官方启动器更新一次。

## 验证记录

同一台机器、同一个游戏版本，只换启动方式做对照：

| 项目 | 直接启动 `Endfield.exe` | 本程序 |
| --- | --- | --- |
| `PC launch auth failed / errorCode 10100` | 出现 | 整份 `Player.log` 里一条都没有 |
| 游戏窗口 | 无，提示「请用官方启动器启动」 | `Endfield`，`Responding = True` |
| 进入渲染 | 否 | 是（`HGCreateSwapchain`） |
| `Endfield.exe` 的父进程 | `explorer.exe` 或 `steam.exe` | 挂起的临时 `Games.exe` |

「两条检查」这个结论是拿同一个夹具（`%TEMP%` 里一份改名为 `Games.exe` 的文件）逐项试出来的：

| 夹具内容 | 结果 |
| --- | --- |
| 本程序自己的 exe（未签名） | `10101` / `2148204800` |
| `notepad.exe`（签名有效，发行商是 Microsoft） | `10101` / `2148204800` |
| 官方 `Launcher.exe` 改名（Shanghai Hypergryph 签名） | 通过，`code: 0` |
| 游戏目录 `PlatformProcess.exe` 改名（同发行商） | 通过，`code: 0` |

现在的默认路径同样留了读数：`OneChannelLC.log` 里是 `Parent process check success:…` +
`{"code":2003,"msg":{"code":0,"msg":{"filePath":"c:\\users\\…\\temp\\endfielddirectlauncher\\<pid>\\games.exe"}}}`
——通过，且父进程路径确实是临时副本。另外单独验过收尾：只杀本程序进程，挂起的 `Games.exe` 会被 job 一起带走，
而游戏本身继续运行。

## 开发

### 构建

```
cargo build --release
```

产物是 `target\release\EndfieldDirectLauncher.exe`。构建时要用 Windows SDK 的 `rc.exe` 把 `assets/app.ico`
编成资源嵌进 exe；`rc.exe` 既不在 PATH 也不在 SDK 默认位置时，设环境变量 `RC` 指过去。

### 测试

```
EFDL_TEST_NO_MANIFEST=1 cargo test
```

exe 清单里写着 `requireAdministrator`，而 `cargo test` 是从普通权限的 shell 拉起测试二进制的，不关掉清单就会在
第一条断言之前以 os error 740 失败。这个环境变量只影响这一次构建，正常 `cargo build` 仍然带管理员清单。

### 项目结构

```
EndfieldDirectLauncher/
├─ src/main.rs          全部逻辑：参数、父进程装配、日志
├─ build.rs             构建期资源：管理员清单、图标
├─ assets/
│  ├─ app.ico           多尺寸图标（由 extract-icon.py 生成）
│  ├─ icon.png          README 用的图标
│  └─ extract-icon.py   从 Endfield.exe 提取图标
├─ Cargo.toml
├─ LICENSE
└─ README.md
```

图标从 `Endfield.exe` 里提取：`assets/extract-icon.py` 解析 PE 资源目录的 `RT_GROUP_ICON` / `RT_ICON`，
还原出 9 档尺寸（256² 到 16²）的 `assets/app.ico`，构建时由 `build.rs` 经 `rc.exe` 嵌进 exe。

## 许可

以 [MIT 协议](LICENSE)开源。`assets/` 里的游戏图标版权归游戏方，不在该协议授权范围内。

绕过官方启动器不是官方支持的行为，使用前请自行判断风险。
