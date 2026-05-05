use normawm::{
    ai::format_ai_window_digest,
    wm::{build_ai_window_digest_from_layout, WindowLayoutSnapshot},
};
use smithay::utils::{Logical, Rectangle, Size};

#[test]
fn wm_window_digest_is_readable_before_ai_ingestion() {
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

    let digest = build_ai_window_digest_from_layout("main", Size::from((1200, 720)), &windows);
    let ai_input = format_ai_window_digest(&digest);

    println!("\n--- AI INPUT PREVIEW ---\n{ai_input}\n------------------------");

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
