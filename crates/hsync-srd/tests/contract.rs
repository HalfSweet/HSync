//! The sigrokdecode contract as DSView's runtime has it, checked with a small decoder of our
//! own: condition-less and skip waits, unbound optional channels, the matched mask, numbers in
//! annotations, type codes and rows.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use hsync_capture::LogicCapture;
use hsync_srd::{DecoderHost, Layer, Stack, run_stack};
use parking_lot::RwLock;

const PROBE: &str = r#"
import sigrokdecode as srd

class Decoder(srd.Decoder):
    api_version = 3
    id = 'hsync_probe'
    name = 'Probe'
    longname = 'Host probe'
    desc = 'Checks the decoder host.'
    license = 'gplv2+'
    inputs = ['logic']
    outputs = ['hsync_probe']
    tags = ['Debug']
    channels = ({'id': 'a', 'name': 'A', 'desc': 'Line A'},)
    optional_channels = ({'id': 'b', 'name': 'B', 'desc': 'Line B'},)
    options = ({'id': 'n', 'desc': 'Skip', 'default': 3},)
    annotations = (('x', 'X'), ('y', 'Y'), ('z', 'Z'))

    def __init__(self):
        self.reset()

    def reset(self):
        self.ended = False

    def start(self):
        self.out = self.register(srd.OUTPUT_ANN)
        assert self.register(srd.OUTPUT_ANN) == self.out

    def decode(self):
        pins = self.wait()
        assert not self.has_channel(1)
        self.put(self.samplenum, self.samplenum, self.out,
                 [0, ['first', str(self.samplenum), str(pins[1])]])
        self.wait({0: 'r'})
        start = self.samplenum
        self.wait({'skip': self.options['n']})
        self.put(start, self.samplenum, self.out, [1, ['@4f']])
        self.wait([{1: 'h'}, {1: 'l'}])
        self.put(self.samplenum, self.samplenum, self.out,
                 [2, ['m={$}', 'm=%d' % self.matched, 7]])
        self.put(0, 0, self.out, ['not', 'an annotation'])
        while True:
            self.wait({0: 'n'})

    def end(self):
        self.put(self.last_samplenum, self.last_samplenum, self.out, [0, ['end']])
"#;

fn decoders() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hsync-srd-contract-{}", std::process::id()));
    let package = dir.join("hsync_probe");
    std::fs::create_dir_all(&package).expect("package");
    std::fs::write(package.join("__init__.py"), "from .pd import Decoder\n").expect("init");
    std::fs::write(package.join("pd.py"), PROBE).expect("pd");
    dir
}

/// A capture whose channel 0 holds `levels`.
fn capture(levels: &str) -> LogicCapture {
    let mut capture = LogicCapture::new(1_000_000, [0]);
    let word = levels.bytes().enumerate().fold(0u64, |word, (bit, level)| {
        word | (u64::from(level == b'1') << bit)
    });
    let data = capture.channel_mut(0).expect("channel");
    data.append_words(&[word]);
    data.truncate(levels.len() as u64);
    capture
}

#[test]
fn keeps_dsviews_contract() {
    let dir = decoders();
    let host = DecoderHost::new(dir.clone());
    let info = host.info("hsync_probe").expect("probe");
    // Two-element classes count up from 7; no annotation rows means one row for all.
    let kinds: Vec<Option<u32>> = info.annotations.iter().map(|class| class.kind).collect();
    assert_eq!(kinds, [Some(7), Some(8), Some(9)]);

    let stack = Stack {
        layers: vec![Layer {
            id: "hsync_probe".into(),
            options: serde_json::Map::new(),
        }],
        channels: [("a".to_owned(), 0)].into(),
    };
    let capture = Arc::new(RwLock::new(capture("0011100000")));
    let outcome = run_stack(
        &host,
        &capture,
        10,
        &stack,
        &Arc::new(AtomicBool::new(false)),
    )
    .expect("run");
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    let layer = &outcome.layers[0];
    assert_eq!(layer.rows, [("hsync_probe".to_owned(), "Probe".to_owned())]);
    let shown: Vec<(u64, u64, Vec<String>)> = layer
        .annotations
        .iter()
        .map(|annotation| (annotation.start, annotation.end, annotation.shown()))
        .collect();
    assert_eq!(
        shown,
        [
            // The first condition-less wait answers sample 0; B is not given: 255.
            (0, 0, vec!["first".into(), "0".into(), "255".into()]),
            // A rises at 2; a skip of 3 after it lands on 5.
            (2, 5, vec!["4F".into()]),
            // B reads low: the second condition holds at 6, mask 0b10; 7 is the value.
            (6, 6, vec!["m=07".into(), "m=2".into()]),
            // end() learns where the samples ended.
            (10, 10, vec!["end".into()]),
        ]
    );
    let _ = std::fs::remove_dir_all(dir);
}
