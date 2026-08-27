//! Generates dist/<name>/index.html for a game.
use maud::{DOCTYPE, html};
use std::path::Path;
use xtask::{
    analytics_bridge, audio_context_capture_script, audio_mute_bridge, base_url,
    daily_challenge_button, daily_mode_query_bridge, description, favicon_links, fullscreen_bridge,
    game_id_bridge, game_json_ld, game_page_info, gtag_head, hotkey_popup, loading_screen,
    manifest_json, native_size, native_size_style, orientation_hint, popup_pause_bridge, pwa_head,
    screenshot_bridge, scroll_cue, session_signals_bridge, share_result_bridge, social_image,
    social_video, stream_mode_query_bridge, sw_register_bridge, title, variant_query_bridge,
};

fn main() {
    let name = std::env::args()
        .nth(1)
        .expect("usage: generate_game_html <name>");
    let dist = Path::new("dist");

    let base_url = base_url();
    let title = title(&name);
    let description = description(&name);
    let page_url = format!("{base_url}{name}/");
    let og = social_image(&base_url, dist, Some(&format!("{name}/preview.png")));
    let video = social_video(&base_url, dist, &name);

    let page = html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                (favicon_links(&base_url, dist))
                title { (title) " — Hotel Chair Games" }
                meta name="description" content=(description);
                link rel="canonical" href=(page_url);
                meta property="og:type" content=(if video.is_some() { "video.other" } else { "website" });
                meta property="og:site_name" content="Hotel Chair Games";
                meta property="og:locale" content="en_US";
                meta property="og:title" content=(format!("{title} — Hotel Chair Games"));
                meta property="og:description" content=(description);
                meta property="og:url" content=(page_url);
                meta property="og:image" content=(og.url);
                meta property="og:image:alt" content=(format!("{title} being played automatically by an AI"));
                meta name="twitter:card" content=(og.twitter_card);
                meta name="twitter:image" content=(og.url);
                @if let Some(video_url) = &video {
                    meta property="og:video" content=(video_url);
                    meta property="og:video:secure_url" content=(video_url);
                    meta property="og:video:type" content="video/mp4";
                    @let (w, h) = native_size(&name);
                    meta property="og:video:width" content=(w.to_string());
                    meta property="og:video:height" content=(h.to_string());
                }
                // Shared across every game page (see `BUNDLE_WASM`) — one URL, one cache entry.
                link rel="preload" href=(format!("../{}", xtask::BUNDLE_WASM)) as="fetch" crossorigin="anonymous";
                (game_json_ld(&base_url, &title, &description, &page_url, &og.url))
                (gtag_head())
                (pwa_head("#000000"))
                (native_size_style(&name))
            }
            body {
                // One full viewport of game, then the content section below the fold —
                // see `native_size_style`'s `.stage` and `game_page_info`.
                div class="stage" {
                    main {
                        (loading_screen())
                        canvas id="glcanvas" tabindex="0" {}
                    }
                    (scroll_cue())
                }
                (game_page_info(dist, &name))
                // Must precede the wasm bundle script — see its own doc comment for why.
                (audio_context_capture_script())
                script src="../mq_js_bundle.js" {}
                (analytics_bridge())
                (audio_mute_bridge())
                (session_signals_bridge(&name))
                (sw_register_bridge("../sw.js"))
                (stream_mode_query_bridge())
                (daily_mode_query_bridge())
                (popup_pause_bridge())
                (screenshot_bridge(&name))
                (share_result_bridge())
                // Every page, not just minesweeper's: the shared binary links in
                // `lib/minesweeper`'s `hcg_initial_variant_is_hex` import whichever game a
                // page runs, and an unregistered import fails instantiation with a LinkError.
                (variant_query_bridge())
                // Which game the shared binary should run — must be registered before
                // `load(...)`, like every other bridge above.
                (game_id_bridge(&name))
                script { (maud::PreEscaped(format!("load(\"../{}\");", xtask::BUNDLE_WASM))) }
                (hotkey_popup(&name))
                (daily_challenge_button())
                (fullscreen_bridge())
                (orientation_hint(&name))
            }
        }
    };

    let dir = dist.join(&name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), page.into_string()).unwrap();
    std::fs::write(
        dir.join("manifest.webmanifest"),
        manifest_json(dist, &title, &description, "#000000", "../"),
    )
    .unwrap();
}
