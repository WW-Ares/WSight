//! 让一个窗口真正"长在桌面上"的 Win32 细节。
//!
//! 三件事，都是 `tauri` 的窗口 API 没有直接给的：
//!
//! 1. **不进任务栏**。`skipTaskbar` 走的是 `ITaskbarList`，任务栏被第三方美化
//!    工具接管后（StartAllBack / 各类 dock），那条记录会被重建成"普通窗口"，
//!    悬浮窗就冒到任务栏上了。`WS_EX_TOOLWINDOW` 是窗口自己的属性：不进任务栏、
//!    不进 Alt+Tab，谁来管任务栏都一样。
//! 2. **点了不抢焦点、不往上浮**。窗口被激活就会升到普通窗口栈顶，于是它会盖住
//!    用户正在用的程序 —— 桌面挂件不该有这种手气。`WS_EX_NOACTIVATE` 让点击
//!    只把消息送进 webview，不激活窗口。进"调整"模式时再摘掉，因为方向键微调
//!    需要键盘焦点。
//! 3. **Win+D 之后还在**。实测（`.verify/probe_wind4.py`）：显示桌面时窗口并没有
//!    被隐藏（`IsWindowVisible` 仍为 True），而是桌面窗口 `Progman` 被抬到了所有
//!    普通窗口之上，把它盖住了。所以只要发现"盖住我的是桌面"，就把自己抬回桌面
//!    之上；平时仍然贴在窗口栈最底下，不碰任何 app。
#![cfg(windows)]

use std::ffi::c_void;
use std::sync::OnceLock;

use tauri::WebviewWindow;

const GWL_EXSTYLE: i32 = -20;
const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
const WS_EX_APPWINDOW: isize = 0x0004_0000;
const WS_EX_NOACTIVATE: isize = 0x0800_0000;
const WS_EX_TOPMOST: isize = 0x0000_0008;

const HWND_BOTTOM: isize = 1;
const HWND_TOPMOST: isize = -1;
const HWND_NOTOPMOST: isize = -2;
const GW_HWNDNEXT: u32 = 2;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;
const SWP_NOOWNERZORDER: u32 = 0x0200;

const GA_ROOT: u32 = 2;
const SW_RESTORE: i32 = 9;

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;

#[derive(Clone, Copy)]
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// `MSG`，只为了让消息循环有个能过 `GetMessageW` 的落点，字段不需要读。
#[repr(C)]
struct Msg {
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: Point,
}

#[link(name = "user32")]
extern "system" {
    fn GetWindowLongPtrW(hwnd: *mut c_void, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
    fn SetWindowPos(
        hwnd: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    fn GetWindowRect(hwnd: *mut c_void, rect: *mut Rect) -> i32;
    fn GetClassNameW(hwnd: *mut c_void, buf: *mut u16, max: i32) -> i32;
    fn WindowFromPoint(pt: Point) -> *mut c_void;
    fn GetAncestor(hwnd: *mut c_void, flags: u32) -> *mut c_void;
    fn IsWindowVisible(hwnd: *mut c_void) -> i32;
    fn IsIconic(hwnd: *mut c_void) -> i32;
    fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
    fn GetTopWindow(hwnd: *mut c_void) -> *mut c_void;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindow(hwnd: *mut c_void, cmd: u32) -> *mut c_void;
    fn GetWindowTextLengthW(hwnd: *mut c_void) -> i32;
    fn SetWinEventHook(
        event_min: u32,
        event_max: u32,
        module: *mut c_void,
        proc: Option<unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, i32, u32, u32)>,
        pid: u32,
        tid: u32,
        flags: u32,
    ) -> *mut c_void;
    fn UnhookWinEvent(hook: *mut c_void) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn SetWindowsHookExW(
        id_hook: i32,
        proc: Option<unsafe extern "system" fn(i32, usize, isize) -> isize>,
        module: *mut c_void,
        thread_id: u32,
    ) -> *mut c_void;
    fn UnhookWindowsHookEx(hook: *mut c_void) -> i32;
    fn CallNextHookEx(hook: *mut c_void, code: i32, w_param: usize, l_param: isize) -> isize;
}

fn handle(win: &WebviewWindow) -> Option<*mut c_void> {
    win.hwnd().ok().map(|h| h.0 as *mut c_void)
}

/// 把窗口标记成"工具窗口"，顺手清掉 `WS_EX_APPWINDOW` —— 后者是任务栏收它的
/// 通行证，两个样式同时存在时任务栏按 `APPWINDOW` 处理，`TOOLWINDOW` 白设。
pub fn mark_as_tool(win: &WebviewWindow) {
    let Some(hwnd) = handle(win) else {
        return;
    };
    // SAFETY: 窗口句柄来自 Tauri 自己的窗口；只改扩展样式位，再让系统重算边框。
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let next = (style | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW;
        if next != style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
            );
        }
    }
}

/// 点击不激活窗口 —— 于是它不会被顶到别的程序上面去。
pub fn set_no_activate(win: &WebviewWindow, on: bool) {
    let Some(hwnd) = handle(win) else {
        return;
    };
    // SAFETY: 同上，只动扩展样式的一个位。
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let next = if on {
            style | WS_EX_NOACTIVATE
        } else {
            style & !WS_EX_NOACTIVATE
        };
        if next != style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
        }
    }
}

/// 被系统藏起来了吗（隐藏或被最小化）。
///
/// 注意不能只看"被盖住"：`Win+D` 之后窗口是可见的，只是被桌面压住了，
/// 那是另一个分支。
/// 窗口被判成"图标化"了吗（`IsIconic`）。
///
/// `Win+D` 在不同 Windows 版本上的实现不一样：本机 Win11 26100 是**把 `Progman`
/// 抬到所有窗口之上**（`.verify/exp_wind4_minimize.py` 对照实验：普通窗口 / 工具
/// 窗口 / 卡片全都没被最小化、也没被 cloak），所以只要临时置顶就能活下来。但
/// Win10 时代它是真给窗口发 `SC_MINIMIZE`。卡片本身没有任何最小化入口，一旦
/// `IsIconic` 为真就只能是系统干的 —— 那就得显式叫回来。
pub fn is_minimized(win: &WebviewWindow) -> bool {
    let Some(hwnd) = handle(win) else {
        return false;
    };
    // SAFETY: 只读查询。
    unsafe { IsIconic(hwnd) != 0 }
}

/// 把被最小化的窗口叫回来。
///
/// **必须 `SW_RESTORE`**：`ShowWindow(SW_SHOW)` 对已经最小化的窗口不恢复（只是
/// 让它在最小化状态下"可见"），`verify_restore_minimized.py` 实测过 —— 只靠
/// tao 的 `win.show()` 会一直卡在 `IsIconic=1`，守护线程每 250ms 白喊一次。
pub fn restore_window(win: &WebviewWindow) {
    let Some(hwnd) = handle(win) else {
        return;
    };
    // SAFETY: 只改窗口的显示状态。
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
    }
}

pub fn hidden_by_shell(win: &WebviewWindow) -> bool {
    let Some(hwnd) = handle(win) else {
        return false;
    };
    // SAFETY: 只读查询。
    unsafe { IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 }
}

fn class_name(hwnd: *mut c_void) -> String {
    let mut buf = [0u16; 128];
    // SAFETY: 缓冲区长度随指针一起传，最多写 128 个 UTF-16。
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

/// 窗口正中间那个像素，现在最上面的是谁？
///
/// 返回 `Some(true)` = 被桌面盖住了。桌面有两种：`Progman`（无壁纸轮播时）
/// 和壁纸宿主 `WorkerW`（轮播 / 动态壁纸时），两种都算。
pub fn covered_by_desktop(win: &WebviewWindow) -> bool {
    let Some(hwnd) = handle(win) else {
        return false;
    };
    let mut rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: 只读查询，rect 是栈上的合法缓冲。
    let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
    if ok == 0 {
        return false;
    }
    let pt = Point {
        x: (rect.left + rect.right) / 2,
        y: (rect.top + rect.bottom) / 2,
    };
    // SAFETY: 只读查询；Point 是 8 字节 POD，按值传参在 x64 上就是寄存器。
    let hit = unsafe { WindowFromPoint(pt) };
    if hit.is_null() {
        return false;
    }
    // SAFETY: 只读查询。
    let root = unsafe { GetAncestor(hit, GA_ROOT) };
    if root.is_null() || root == hwnd {
        return false;
    }
    // 桌面图标是 SHELLDLL_DefView 里的 SysListView32，往上找根窗口同样落到
    // Progman，所以这里按根窗口的类名判断就够了。
    matches!(class_name(root).as_str(), "Progman" | "WorkerW")
}

/// 桌面是不是正压在所有普通窗口上面（= 用户刚按了 Win+D）。
///
/// 判定方式不看前台窗口，也能在我们自己已经置顶时给出正确答案：从窗口栈顶
/// 往下走，跳过隐藏的、置顶层的、没有标题的，以及 `ignore` 里的窗口（我们自己的
/// 两个悬浮窗），第一个剩下的普通窗口是不是桌面。
///
/// 为什么必须跳置顶层：显示桌面时我们把悬浮窗抬到了置顶，它排在最前面，
/// 不跳过的话第一个看到的永远是它自己。
pub fn desktop_on_top(ignore: &[isize]) -> bool {
    let mut h = unsafe { GetTopWindow(std::ptr::null_mut()) };
    while !h.is_null() {
        // SAFETY: 全部是只读查询，h 来自上面的 z 序枚举。
        let (visible, ex, has_title) = unsafe {
            (
                IsWindowVisible(h) != 0,
                GetWindowLongPtrW(h, GWL_EXSTYLE),
                GetWindowTextLengthW(h) > 0,
            )
        };
        let mine = ignore.iter().any(|x| *x == h as isize);
        if visible && !mine && has_title {
            // 桌面按类名相认，**不看它自己的置顶位**：Win+D 之后 `Progman` 常被
            // 留在置顶层，按"跳过置顶窗口"走就永远遇不到它，"桌面在最前面"
            // 这个判断会一直是 false —— 这才是恢复后卡片不肯落回的源头。
            if matches!(class_name(h).as_str(), "Progman" | "WorkerW") {
                return true;
            }
            // 置顶层的有标题窗口是浮层 / 置顶程序，跳过继续往下找。
            if ex & WS_EX_TOPMOST == 0 {
                return false;
            }
        }
        // SAFETY: 同上，沿着 z 序往下走一格。
        h = unsafe { GetWindow(h, GW_HWNDNEXT) };
    }
    false
}

/// 前台窗口的类名，只给日志用。
pub fn foreground_class() -> String {
    // SAFETY: 只读查询。
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_null() {
        return String::from("-");
    }
    class_name(fg)
}

/// 桌面此刻是不是真的**占着前台**——用户正在看的那一层就是桌面。
///
/// 只看 z 序会被"全屏 / 置顶的程序"骗：那类窗口不在普通层里（预览器一进全屏就把
/// 自己抬成置顶、标题还可能为空），`desktop_on_top` 会把它们整排跳过，一路走到
/// `Progman`，得出"桌面还在最前面"的结论。于是守护线程不停地给卡片续临时置顶，
/// 卡片就一直压在用户刚恢复的那个预览器上面 —— 这就是"被卡片压住几秒"。
///
/// 前台窗口是唯一不会骗人的信号：真正在用的那个窗口才在前台。所以判据是
/// 「前台就是桌面」**且**「桌面确实盖在所有普通窗口之上」。
pub fn desktop_showing(ignore: &[isize]) -> bool {
    // SAFETY: 只读查询。
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_null() || ignore.iter().any(|x| *x == fg as isize) {
        return false;
    }
    if !matches!(class_name(fg).as_str(), "Progman" | "WorkerW") {
        return false;
    }
    desktop_on_top(ignore)
}

/// 抬到置顶层 —— **只在桌面把窗口盖住的那一瞬间用**。
///
/// 实测（`.verify/exp_wind_d4.py`，Win11 26100）：显示桌面时 Windows 把
/// `Progman` 抬到所有普通窗口之上，贴底的窗口就被盖住了；而置顶窗口不受影响。
/// `SetParent` 到桌面（DeskBox 的做法）会把坐标系换成父窗口的客户区，还要在
/// Explorer 重启时重新挂一次，对一个已经成熟的拖拽/磁吸实现来说代价太大。
/// 所以只用这一下：桌面浮上来时置顶，桌面收起后立刻落回底部。
pub fn set_topmost(win: &WebviewWindow) {
    let Some(hwnd) = handle(win) else {
        return;
    };
    // SAFETY: 只动 z 序层级，不移动不激活。
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST as *mut c_void,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

/// 桌面窗口（`Progman`，没有就退而求其次找 `WorkerW`）、它**上面**那个普通
/// 窗口，以及"我已经在这一串里了"。
///
/// 第二项是"桌面正上方"的锚点：把悬浮窗插到它下面，窗口就落在桌面之上、所有
/// 普通窗口之下。`ignore` 是同一个进程里的**其他**悬浮窗 —— 它们同样贴在桌面
/// 正上方，算锚点时必须跳过，否则两张卡片会把彼此当成锚点来回搬运（A 说"我要
/// 贴桌面，所以我要搬到 B 下面"，B 说"我要贴桌面，所以我要搬到 A 下面"），每次
/// `SetWindowPos` 都是一次重合成，肉眼看到的就是持续轻闪。
///
/// 第三项解决同一个问题的另一半：我本来就夹在"桌面正上方那一串"里时不用再动，
/// 哪怕旁边还站着另一张卡片。找不到桌面时返回 `None`。
fn desktop_slot(
    hwnd: *mut c_void,
    ignore: &[isize],
) -> Option<(*mut c_void, *mut c_void, bool)> {
    for wanted in ["Progman", "WorkerW"] {
        // 从栈顶往下走。锚点只认普通层窗口：置顶的那一拨是任务栏、输入法
        // 这类浮层，把卡片插到它们下面会让卡片也变成置顶的。
        let mut h = unsafe { GetTopWindow(std::ptr::null_mut()) };
        let mut anchor: *mut c_void = std::ptr::null_mut();
        while !h.is_null() {
            // SAFETY: 全部只读查询，h 来自上面的 z 序枚举。
            let visible = unsafe { IsWindowVisible(h) != 0 };
            if visible {
                // **桌面按类名相认，不看它自己的置顶位。** Win+D 之后 Explorer
                // 常把 `Progman` 留在置顶层（现场 `.verify/diag_live_z.py`：
                // Progman top=Y 却排在栈底）。这个位是残留、不影响显示，但按
                // \"跳过置顶窗口\"的规则去找桌面就再也找不到 —— 于是落位退化成
                // `to_bottom`，而 `HWND_BOTTOM` 对置顶窗口完全无效：卡片留在
                // 置顶层压着用户刚恢复的程序，守护线程每 250ms 白试一次。
                if class_name(h) == wanted {
                    let placed = !is_topmost_hwnd(hwnd) && sits_on_desktop(hwnd, h, ignore);
                    return Some((h, anchor, placed));
                }
                if h != hwnd
                    && !is_topmost_hwnd(h)
                    && !ignore.iter().any(|x| *x == h as isize)
                {
                    anchor = h;
                }
            }
            // SAFETY: 沿着 z 序往下走一格。
            h = unsafe { GetWindow(h, GW_HWNDNEXT) };
        }
    }
    None
}

/// 卡片（普通层）正下方第一个能看见的窗口是不是桌面 —— 即「紧贴桌面上方」。
///
/// 同进程的另一张卡片跳过：它俩互为邻居，谁在下面都不算挡路。
fn sits_on_desktop(hwnd: *mut c_void, desktop: *mut c_void, ignore: &[isize]) -> bool {
    let mut h = unsafe { GetWindow(hwnd, GW_HWNDNEXT) };
    while !h.is_null() {
        // SAFETY: 只读查询，沿 z 序往下走。
        if unsafe { IsWindowVisible(h) != 0 } {
            if h == desktop {
                return true;
            }
            if !ignore.iter().any(|x| *x == h as isize) {
                return false;
            }
        }
        // SAFETY: 同上。
        h = unsafe { GetWindow(h, GW_HWNDNEXT) };
    }
    false
}

fn is_topmost_hwnd(hwnd: *mut c_void) -> bool {
    // SAFETY: 只读查询。
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST != 0 }
}

/// 把窗口放回"桌面家具"该在的那一层 —— **紧贴桌面上方**。
///
/// 不能简单用 `HWND_BOTTOM`。桌面本身也会动：按 `Win+D` 时它被抬到最上面，
/// 恢复时从高处落回去。落回过程的那几帧里，压到"全栈最底"的窗口会跑到桌面
/// **下面**，于是卡片整块消失一帧 —— 这就是恢复时那一下闪。
///
/// 插到"桌面上方那个窗口"的下面，位置就与桌面同步：桌面在半空时卡片在它正
/// 上方，桌面落地时卡片仍在它正上方，永远不会被桌面盖住。
///
/// 落位前要自己把 `WS_EX_TOPMOST` 摘掉 —— 别指望"插到普通窗口下面"能顺带清位，
/// 实测那样只会把置顶窗口摆到置顶层内部靠下的位置（见函数里的注释）。
pub fn place_at_desktop_level(win: &WebviewWindow, ignore: &[isize]) -> bool {
    let Some(hwnd) = handle(win) else {
        return false;
    };
    let was_top = is_topmost_hwnd(hwnd);
    // 桌面正占着前台（`Win+D` 还没收起）时不许摘置顶位：摘了就是交给桌面盖住。
    // 这种时候"卡片在置顶层"是正确状态，抬高那条路会负责记账。
    if was_top && desktop_showing(ignore) {
        return false;
    }
    let Some((_desktop, anchor, placed)) = desktop_slot(hwnd, ignore) else {
        // 找不到桌面（极少见）：退回原来的最底层，至少把置顶位摘掉。
        to_bottom(win);
        return true;
    };
    // 已经贴在桌面正上方、也没挂着置顶位，不用再动。挪一下只会多一次重合成。
    if placed && !was_top {
        return false;
    }
    // **摘置顶位必须显式做。** 实测（`.verify/verify_untopmost.py`，Win11 26100）：
    // 把带 `WS_EX_TOPMOST` 的窗口 `SetWindowPos` 到普通窗口下面，Windows 只是把
    // 它摆到置顶层内部靠下的位置，样式位原封不动 —— 卡片于是永远压在普通程序
    // 之上，守护线程每 250ms 白落一次。先 `HWND_NOTOPMOST` 掉出置顶层，再按锚点
    // 插入；两次调用在同一个 tick 里完成，DWM 合成时只会看到最终位置。
    if was_top {
        // SAFETY: 只改 z 序层级，不移动不激活。
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_NOTOPMOST as *mut c_void,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }
    if anchor.is_null() || anchor == hwnd {
        // 桌面上方没有任何普通窗口（`Win+D` 那一下）：没有位置可插，摘掉置顶位
        // 就到这一步为止 —— 接着由"桌面占着前台"那条分支重新抬起来。
        return was_top;
    }
    // SAFETY: 只改 z 序。
    unsafe {
        SetWindowPos(
            hwnd,
            anchor,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
    true
}

/// 窗口身上现在挂着 `WS_EX_TOPMOST` 吗。
///
/// 兜底校准用：临时置顶是"桌面在最前面"期间才成立的借据，桌面一收就得还。
/// 状态位可能因为丢事件、全屏程序抢层级、shell 动画而错乱，所以判断时只认
/// 窗口上的**实际样式位**，不认我们记的那几个 `AtomicBool`。
pub fn is_topmost(win: &WebviewWindow) -> bool {
    let Some(hwnd) = handle(win) else {
        return false;
    };
    // SAFETY: 只读查询。
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST != 0 }
}

/// 贴回窗口栈最底下（桌面家具的那一层）。
pub fn to_bottom(win: &WebviewWindow) {
    let Some(hwnd) = handle(win) else {
        return;
    };
    // SAFETY: 同 `zorder::to_bottom`，只改 z 序。
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_BOTTOM as *mut c_void,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

// ------------------------------------------------------- foreground watching
//
// 贴底的窗口在 `Win+D` 那一刻会被桌面盖住。靠每 700ms 的轮询再抬高，用户
// 看到的是"卡片消失一下，然后又回来"—— 那一闪是轮询间隔造成的，不是系统的
// 效果。前台窗口变化事件在桌面被抬起的同一瞬间就发出来，毫秒级跟上，就没有
// 可看的那一下了；轮询仍然留着，只当兜底（钩子掉了、事件丢了）。

static FOREGROUND_SINK: OnceLock<fn(bool)> = OnceLock::new();

unsafe extern "system" fn foreground_proc(
    _hook: *mut c_void,
    event: u32,
    hwnd: *mut c_void,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || hwnd.is_null() {
        return;
    }
    // 桌面的两种宿主都算：`Progman`（一般情况）、`WorkerW`（壁纸轮播/动态壁纸）。
    let on_desktop = matches!(class_name(hwnd).as_str(), "Progman" | "WorkerW");
    if let Some(sink) = FOREGROUND_SINK.get() {
        sink(on_desktop);
    }
}

/// 订阅"桌面跑到最前面"和"桌面又收起"这两个瞬间。
///
/// `sink(true)` = 现在盖在最上面的是桌面（用户刚按了 `Win+D`）；
/// `sink(false)` = 有别的窗口接管了前台。
///
/// 事件由系统投递到注册线程的消息循环里，所以这里自带一个线程和一个
/// `GetMessageW` 循环，不占用 Tauri 的主循环，也不和它抢消息。
pub fn watch_foreground(sink: fn(bool)) {
    if FOREGROUND_SINK.set(sink).is_err() {
        return; // 已经有人订阅了，重复注册没有意义
    }
    std::thread::spawn(|| {
        // SAFETY: 钩子只监听一个事件，回调不碰任何未初始化内存；消息循环
        // 的唯一作用就是让系统能把事件送进来。
        unsafe {
            let hook = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                std::ptr::null_mut(),
                Some(foreground_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if hook.is_null() {
                return;
            }
            let mut msg = std::mem::zeroed::<Msg>();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            UnhookWinEvent(hook);
        }
    });
}

// ------------------------------------------------------------- click watching
//
// 右击弹出的菜单是网页里的一个浮层，收起它靠的是"窗口失去焦点"。可悬浮窗带着
// `WS_EX_NOACTIVATE`（点它不抢焦点，见文件头第 2 点），于是它从来没有焦点可丢：
// 点桌面、点任务栏、点别的程序，都不会产生 `blur`，菜单就那样一直挂在屏幕上。
//
// 底层鼠标钩子补上这个信号 —— 系统在把点击投递出去之前，会先问一遍所有
// `WH_MOUSE_LL` 钩子，无论点是落在桌面、任务栏还是别的程序上都绕不过去。
// 回调里只做一次 `WindowFromPoint` 和一次函数调用，不做任何阻塞动作。

const WH_MOUSE_LL: i32 = 14;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_MBUTTONDOWN: u32 = 0x0207;

/// `MSLLHOOKSTRUCT`；只读 `pt`，其余字段照原样占位。
#[repr(C)]
struct MsllHookStruct {
    pt: Point,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra: usize,
}

static CLICK_SINK: OnceLock<fn(isize)> = OnceLock::new();

unsafe extern "system" fn mouse_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && l_param != 0 {
        let msg = w_param as u32;
        if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
            // SAFETY: 底层钩子保证 code >= 0 时 l_param 指向一个 MSLLHOOKSTRUCT。
            let info = &*(l_param as *const MsllHookStruct);
            let hit = WindowFromPoint(info.pt);
            let root = if hit.is_null() {
                std::ptr::null_mut()
            } else {
                GetAncestor(hit, GA_ROOT)
            };
            if let Some(sink) = CLICK_SINK.get() {
                sink(root as isize);
            }
        }
    }
    // SAFETY: 转交给链上的下一个钩子 —— 别的程序（输入法、手势工具）也可能在听鼠标。
    CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param)
}

/// 订阅"用户按下鼠标"。回调参数是被点中的**顶层**窗口句柄，由调用方判断这次
/// 点击落在谁身上 —— 桌面、任务栏和别的程序各有各的句柄。
///
/// 钩子是全局的、进程外的（`WH_MOUSE_LL` 不需要注入），但系统对回调有超时限制，
/// 所以这里只做只读查询；真正的处理放在订阅者那边（发一个事件给前端）。
pub fn watch_click_outside(sink: fn(isize)) {
    if CLICK_SINK.set(sink).is_err() {
        return; // 已经订阅过了
    }
    std::thread::spawn(|| {
        // SAFETY: 底层钩子的模块句柄传空是 WinXP 之后的正常写法；回调不碰任何
        // 未初始化内存，也不阻塞系统投递。
        unsafe {
            let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), std::ptr::null_mut(), 0);
            if hook.is_null() {
                return;
            }
            let mut msg = std::mem::zeroed::<Msg>();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            UnhookWindowsHookEx(hook);
        }
    });
}
