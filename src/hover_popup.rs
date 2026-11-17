//! A passive, screen-sized hover surface owned by a library window.
use crate::Message;
use serde_json::{Value, json};
use tao::{
    dpi::LogicalSize,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{WebView, WebViewBuilder};

pub struct HoverPopup {
    // Drop the webview before its window.
    webview: WebView,
    window: Window,
    ready: bool,
    revision: u64,
    content: Option<Value>,
    anchor: [f64; 2],
}

impl HoverPopup {
    pub fn new(
        target: &EventLoopWindowTarget<Message>,
        proxy: EventLoopProxy<Message>,
        owner: WindowId,
    ) -> Result<Self, String> {
        let window = WindowBuilder::new()
            .with_title("Switchboard — Hover")
            .with_inner_size(LogicalSize::new(400., 100.))
            .with_decorations(false)
            .with_resizable(false)
            .with_focused(false)
            .with_focusable(false)
            .with_always_on_top(true)
            .with_visible(false)
            .build(target)
            .map_err(|e| e.to_string())?;
        window
            .set_ignore_cursor_events(true)
            .map_err(|e| e.to_string())?;
        let style = include_str!("ui.html")
            .split_once("<style>")
            .unwrap()
            .1
            .split_once("</style>")
            .unwrap()
            .0;
        let webview = WebViewBuilder::new()
            .with_html(include_str!("hover.html").replace("/* SHARED_STYLE */", style))
            .with_navigation_handler(|url| url == "about:blank")
            .with_ipc_handler(move |request| {
                if request.uri() != "about:blank" {
                    return;
                }
                let Ok(value) = serde_json::from_str::<Value>(request.body()) else {
                    return;
                };
                if value["ready"] == true {
                    let _ = proxy.send_event(Message::HoverReady(owner));
                } else if let (Some(revision), Some(height)) =
                    (value["revision"].as_u64(), value["height"].as_f64())
                {
                    let _ = proxy.send_event(Message::HoverSize(owner, revision, height));
                }
            })
            .build(&window)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            webview,
            window,
            ready: false,
            revision: 0,
            content: None,
            anchor: [0.; 2],
        })
    }

    pub fn ready(&mut self) {
        self.ready = true;
        self.render();
    }

    pub fn show(
        &mut self,
        html: &str,
        appearance: &switchboard::model::Appearance,
        anchor: [f64; 2],
    ) {
        self.revision += 1;
        self.anchor = anchor;
        self.content = Some(json!({"revision":self.revision,"html":html,"appearance":appearance}));
        self.render();
    }

    fn render(&self) {
        if self.ready
            && let Some(content) = &self.content
        {
            let _ = self
                .webview
                .evaluate_script(&format!("window.receiveHover({content})"));
        }
    }

    pub fn resize(&self, owner: &Window, revision: u64, height: f64) {
        // Loading and image callbacks can arrive after dismissal or a new hover.
        if self.content.is_some() && revision == self.revision && height.is_finite() && height > 0.
        {
            crate::platform::place_hover(owner, &self.window, self.anchor, height);
        }
    }

    pub fn verify_smoke_test(&mut self, owner: &Window) -> Result<(), String> {
        let size = self.window.outer_size();
        let owner_size = owner.outer_size();
        if !self.window.is_visible()
            || size.width <= owner_size.width
            || size.height <= owner_size.height
        {
            return Err(format!(
                "Hover did not extend beyond the small library: popup={size:?}, library={owner_size:?}"
            ));
        }
        if !owner.is_focused() || self.window.is_focused() {
            return Err("Hover took keyboard focus from its library".into());
        }
        self.hide();
        self.resize(owner, self.revision, 500.);
        if self.window.is_visible() {
            return Err("A late measurement reopened a dismissed hover".into());
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        self.content = None;
        self.window.set_visible(false);
    }
}

/// Top-left screen coordinates. Prefer below/right, then flip and clamp to the display.
pub fn popup_frame(screen: [f64; 4], anchor: [f64; 2], desired: [f64; 2]) -> [f64; 4] {
    let [sx, sy, sw, sh] = screen;
    let margin = 8f64.min(sw / 2.).min(sh / 2.);
    let width = desired[0].min(sw - 2. * margin).max(1.);
    let height = desired[1].min(sh - 2. * margin).max(1.);
    let right = sx + sw - margin;
    let bottom = sy + sh - margin;
    let x = if anchor[0] + 20. + width <= right {
        anchor[0] + 20.
    } else {
        anchor[0] - 20. - width
    };
    let y = if anchor[1] + 20. + height <= bottom {
        anchor[1] + 20.
    } else {
        anchor[1] - 20. - height
    };
    [
        x.clamp(sx + margin, right - width),
        y.clamp(sy + margin, bottom - height),
        width,
        height,
    ]
}

#[cfg(test)]
mod tests {
    use super::popup_frame;

    #[test]
    fn hover_uses_screen_space_and_flips_at_edges() {
        assert_eq!(
            popup_frame([0., 0., 1440., 900.], [250., 150.], [400., 500.]),
            [270., 170., 400., 500.]
        );
        assert_eq!(
            popup_frame([0., 0., 1440., 900.], [1400., 870.], [400., 500.]),
            [980., 350., 400., 500.]
        );
        // A second display can have negative coordinates and a different size.
        assert_eq!(
            popup_frame([-1280., -200., 1280., 800.], [-20., 550.], [400., 500.]),
            [-440., 30., 400., 500.]
        );
    }

    #[test]
    fn tall_content_and_small_displays_stay_on_screen() {
        assert_eq!(
            popup_frame([0., 0., 320., 240.], [5., 5.], [400., 2000.]),
            [8., 8., 304., 224.]
        );
        let frame = popup_frame([0., 0., 1440., 900.], [700., 650.], [400., 850.]);
        assert_eq!(frame, [720., 8., 400., 850.]);
    }
}
