//! Writes the frontend mock backend's fixtures from the real decoders: the catalog and the
//! demo capture's decoders with every event they put out. Run with `HSYNC_WRITE_FIXTURES=1`
//! after a change to the decoders or the catalog, and commit the result.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hsync_session::{SessionEvent, Workspace};
use serde_json::json;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn write_mock_fixtures() {
    if std::env::var_os("HSYNC_WRITE_FIXTURES").is_none() {
        return;
    }
    let fixtures = root().join("src/ipc/mock/fixtures");
    let catalog = serde_json::to_string_pretty(&hsync_session::catalog()).expect("serializes");
    std::fs::write(fixtures.join("decoder-catalog.json"), catalog + "\n").expect("writes");

    let (sender, events) = crossbeam_channel::unbounded();
    let mut workspace = Workspace::with_events(Arc::new(move |event: SessionEvent| {
        let _ = sender.send(event);
    }));
    let summary = workspace
        .open_demo(&root().join("src-tauri/resources/demo/protocol.dsl"))
        .expect("the demo opens");
    let mut done = 0;
    while done < 4 {
        if let SessionEvent::Decoded { capture_id, .. } = events
            .recv_timeout(Duration::from_secs(20))
            .expect("decoder event")
            && capture_id == summary.id
        {
            done += 1;
        }
    }
    let decoding = workspace.decoding(summary.id).expect("decoders");
    let decoders: Vec<serde_json::Value> = decoding
        .statuses()
        .iter()
        .map(|status| {
            let streams: Vec<serde_json::Value> = status
                .streams
                .iter()
                .map(|stream| {
                    let page = decoding
                        .stream(status.index, stream.index, None, false, 0, u32::MAX)
                        .expect("the stream");
                    json!({
                        "name": stream.name,
                        "kind": stream.kind,
                        "schema": stream.schema,
                        "rows": page.rows,
                    })
                })
                .collect();
            json!({
                "label": status.label,
                "kinds": status.kinds,
                "streams": streams,
            })
        })
        .collect();
    let document = json!({
        "entries": summary.decoders,
        "decoders": decoders,
    });
    std::fs::write(
        fixtures.join("protocol-uiharu.json"),
        serde_json::to_string(&document).expect("serializes") + "\n",
    )
    .expect("writes");
}
