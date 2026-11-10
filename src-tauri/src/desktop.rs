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

use tauri::WebviewWindow;

const GWL_EXSTYLE: i32 = -20;
const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
const WS_EX_APPWINDOW: isize = 0x0004_0000;
const WS_EX_NOACTIVATE: isize = 0x0800_0000;
const WS_EX_TOPMOST: isize = 0x0000_0008;

const HWND_TOP: isize = 0;
const HWND_BOTTOM: isize = 1;
const HWND_TOPMOST: isize = -1;
const GW_HWNDNEXT: u32 = 2;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;
const SWP_NOOWNERZORDER: u32 = 0x0200;

const GA_ROOT: u32 = 2;

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
    fn GetTopWindow(hwnd: *mut c_void) -> *mut c_void;
    fn GetWindow(hwnd: *mut c_void, cmd: u32) -> *mut c_void;
    fn GetWindowTextLengthW(hwnd: *mut c_void) -> i32;
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
        if visible && !mine && has_title && ex & WS_EX_TOPMOST == 0 {
            return matches!(class_name(h).as_str(), "Progman" | "WorkerW");
        }
        // SAFETY: 同上，沿着 z 序往下走一格。
        h = unsafe { GetWindow(h, GW_HWNDNEXT) };
    }
    false
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

/// 抬到普通窗口栈顶 —— 只在"被桌面盖住"时用，那个时刻其他窗口都被最小化了。
pub fn raise_above_desktop(win: &WebviewWindow) {
    let Some(hwnd) = handle(win) else {
        return;
    };
    // SAFETY: 只改 z 序，不动位置尺寸，也不激活。
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOP as *mut c_void,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
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
