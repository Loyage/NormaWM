//! AI 窗口摘要的集成测试。
//!
//! 这项测试不启动真实 Wayland client，而是直接构造布局快照，
//! 验证“窗口管理层 -> AI 文本输入”这条前向路径是否能输出可读内容。

use normawm::{
    ai::format_ai_window_digest,
    wm::{build_ai_window_digest_from_layout, WindowLayoutSnapshot},
};
use smithay::utils::{Logical, Rectangle, Size};

/// 验证窗口摘要在进入 AI 前已经是可读、稳定、可断言的文本格式。
#[test]
fn wm_window_digest_is_readable_before_ai_ingestion() {
    // 手工构造两扇窗口的布局快照。
    // 这样测试不依赖真实 Wayland client，就能专注验证“摘要生成”这条前向链路。
    let windows = vec![
        WindowLayoutSnapshot {
            window_id: "window-1".to_string(),
            role: "xdg_toplevel",
            title: Some("NormaWM Test Window".to_string()),
            app_id: Some("normawm.test-window".to_string()),
            focused: true,
            geometry: Rectangle::<i32, Logical>::new((24, 24).into(), (1152, 328).into()),
        },
        WindowLayoutSnapshot {
            window_id: "window-2".to_string(),
            role: "xdg_toplevel",
            title: Some("Secondary Window".to_string()),
            app_id: Some("normawm.secondary".to_string()),
            focused: false,
            geometry: Rectangle::<i32, Logical>::new((24, 368).into(), (1152, 328).into()),
        },
    ];

    // 把窗口布局转换成 AI 摘要，再格式化成最终要展示/发送的纯文本输入。
    let digest = build_ai_window_digest_from_layout("main", Size::from((1200, 720)), &windows);
    let ai_input = format_ai_window_digest(&digest);

    // `-- --nocapture` 运行测试时，这段输出会直接显示在终端里，
    // 方便人工检查“如果现在把状态送给 AI，它实际会看到什么”。
    println!("\n--- AI INPUT PREVIEW ---\n{ai_input}\n------------------------");

    // 以下断言的目标不是检查每个字符，而是锁住这段输入里最重要的语义要素：
    // 工作区、窗口数量、窗口标识、标题、app_id、焦点和几何信息都必须可读。
    assert!(ai_input.contains("NormaWM window digest for AI"));
    assert!(ai_input.contains("workspace: main"));
    assert!(ai_input.contains("window_count: 2"));
    assert!(ai_input.contains("id=window-1"));
    assert!(ai_input.contains("title=NormaWM Test Window"));
    assert!(ai_input.contains("app_id=normawm.test-window"));
    assert!(ai_input.contains("focused=true"));
    assert!(ai_input.contains("geometry=(24, 24) 1152x328"));
    assert!(ai_input.contains("id=window-2"));
}
