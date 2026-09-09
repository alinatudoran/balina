//! Pure SVG generator: renders the diagram exactly as the canvas shows it
//! (same scene geometry, same colors, same belief/EU values), minus
//! UI-only state (selection, hover, link handles, dot grid).

use std::collections::HashMap;
use std::fmt::Write;

use bn_core::inference::Finding;
use bn_core::io::DisplayMode;
use bn_core::model::{NodeId, NodeKind};
use bn_session::{Document, EngineBridge};

use crate::canvas::edge::EDGE_STROKE;
use crate::canvas::geometry::Rect;
use crate::canvas::node::{belief_row_display, kind_body_color, kind_header_color};
use crate::canvas::scene::{build_scene, NoteOverrides};
use crate::canvas::sticky::{note_bar_color, note_fill, note_plain_text};
use crate::logic::format::expected_value;

pub const PADDING: f64 = 24.0;

const FONT_SANS: &str = "Helvetica Neue, Segoe UI, Roboto, Arial, sans-serif";
const FONT_MONO: &str = "Menlo, Consolas, DejaVu Sans Mono, monospace";
// Tailwind neutral shades as hex (resvg does not parse oklch()).
const TEXT_DARK: &str = "#171717"; // neutral-900
const TEXT_MONO: &str = "#262626"; // neutral-800
const TRACK_BG: &str = "#f5f5f5"; // neutral-100
const TRACK_BORDER: &str = "#d4d4d4"; // neutral-300

/// Full-diagram SVG at committed positions. `None` when the document has
/// nothing to draw (no nodes and no notes).
pub fn network_svg(doc: &Document, bridge: &EngineBridge) -> Option<String> {
    let scene = build_scene(doc, &HashMap::new(), &NoteOverrides::default());
    let b = scene.bounds()?;
    let (width, height) = (b.w + 2.0 * PADDING, b.h + 2.0 * PADDING);

    let mut s = String::with_capacity(4096);
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"{FONT_SANS}\">\n\
         <rect width=\"{width:.0}\" height=\"{height:.0}\" fill=\"white\"/>\n\
         <defs><marker id=\"bn-arrow\" markerWidth=\"9\" markerHeight=\"7\" refX=\"8\" refY=\"3\" \
         orient=\"auto\" markerUnits=\"userSpaceOnUse\">\
         <path d=\"M0,0 L0,6 L8,3 z\" fill=\"{EDGE_STROKE}\"/></marker></defs>\n\
         <g transform=\"translate({tx:.1}, {ty:.1})\">\n",
        tx = PADDING - b.x,
        ty = PADDING - b.y,
    );

    // Edges under nodes, like the canvas edge layer.
    for e in &scene.edges {
        let dash = if e.informational { " stroke-dasharray=\"8 5\"" } else { "" };
        let _ = writeln!(
            s,
            "<path d=\"M {:.1} {:.1} L {:.1} {:.1}\" fill=\"none\" stroke=\"{EDGE_STROKE}\" \
             stroke-width=\"1.5\"{dash} marker-end=\"url(#bn-arrow)\"/>",
            e.from.x, e.from.y, e.to.x, e.to.y,
        );
    }

    for n in &scene.nodes {
        write_node(&mut s, doc, bridge, n.id, n.rect);
    }

    // Sticky notes last: they float in front of the network on the canvas.
    for n in &scene.notes {
        if let Some(note) = doc.notes.get(n.id) {
            write_sticky(&mut s, note, n.rect);
        }
    }

    s.push_str("</g>\n</svg>\n");
    Some(s)
}

/// A sticky note: body, darker top bar, greedily word-wrapped text. Note
/// text is markdown — exported as extracted plain text (markers dropped);
/// rendering HTML inside SVG is not attempted. The wrap uses the same
/// 0.55em character budget as `fit()` — close to the canvas's CSS wrap,
/// exact parity not attempted. A collapsed note is just its bar with the
/// first text line (the scene already gives it the bar rect).
fn write_sticky(s: &mut String, note: &bn_session::Note, r: Rect) {
    let fill = note_fill(note.color);
    let bar = note_bar_color(note.color);
    let plain = note_plain_text(&note.text);
    if note.collapsed {
        let first = plain.lines().next().unwrap_or("");
        let _ = write!(
            s,
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.0}\" height=\"{h:.0}\" rx=\"3\" \
             fill=\"{bar}\" stroke=\"#000000\" stroke-opacity=\"0.15\"/>\n\
             <text x=\"{tx:.1}\" y=\"{ty:.1}\" font-size=\"10\" fill=\"{TEXT_DARK}\">{}</text>\n",
            esc(&fit(first, r.w - 24.0, 10.0)),
            x = r.x,
            y = r.y,
            w = r.w,
            h = r.h,
            tx = r.x + 8.0,
            ty = r.y + r.h / 2.0 + 0.35 * 10.0,
        );
        return;
    }
    let _ = write!(
        s,
        "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.0}\" height=\"{h:.0}\" rx=\"3\" \
         fill=\"{fill}\" stroke=\"#000000\" stroke-opacity=\"0.15\"/>\n\
         <path d=\"M {sx:.1} {y:.1} H {ex:.1} A 3 3 0 0 1 {x1:.1} {ry:.1} V {by:.1} H {x:.1} \
         V {ry:.1} A 3 3 0 0 1 {sx:.1} {y:.1} Z\" fill=\"{bar}\"/>\n",
        x = r.x,
        y = r.y,
        w = r.w,
        h = r.h,
        sx = r.x + 3.0,
        ex = r.x + r.w - 3.0,
        x1 = r.x + r.w,
        ry = r.y + 3.0,
        by = r.y + 14.0,
    );

    let font = note.font_size as f64;
    let line_h = font * 1.35;
    let mut y = r.y + 14.0 + line_h; // baseline of the first line
    for line in wrap_note_text(&plain, r.w - 16.0, font) {
        if y > r.y + r.h - 4.0 {
            break; // overflow is clipped on the canvas; drop it here too
        }
        let _ = writeln!(
            s,
            "<text x=\"{:.1}\" y=\"{y:.1}\" font-size=\"{font:.1}\" fill=\"{TEXT_DARK}\">{}</text>",
            r.x + 8.0,
            esc(&line),
        );
        y += line_h;
    }
}

/// Split on newlines, then greedily wrap each paragraph on spaces to the
/// `fit()` character budget (hard-splitting single over-budget words).
fn wrap_note_text(text: &str, max_px: f64, font_px: f64) -> Vec<String> {
    let budget = (max_px / (font_px * 0.55)).floor().max(1.0) as usize;
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut line_len = 0usize;
        for word in para.split(' ') {
            let mut word = word;
            let mut word_len = word.chars().count();
            // Hard-split words longer than a whole line.
            while word_len > budget {
                if line_len > 0 {
                    out.push(std::mem::take(&mut line));
                    line_len = 0;
                }
                let cut = word.char_indices().nth(budget).map(|(i, _)| i).unwrap_or(word.len());
                out.push(word[..cut].to_string());
                word = &word[cut..];
                word_len = word.chars().count();
            }
            let sep = if line_len > 0 { 1 } else { 0 };
            if line_len + sep + word_len > budget {
                out.push(std::mem::take(&mut line));
                line_len = 0;
            } else if sep == 1 {
                line.push(' ');
                line_len += 1;
            }
            line.push_str(word);
            line_len += word_len;
        }
        out.push(line);
    }
    out
}

/// Mirror of `canvas::node::read_node_data`, but a pure function of the
/// document and bridge instead of the `SESSION` signal.
struct NodeRender {
    title: String,
    kind: NodeKind,
    display: DisplayMode,
    state_names: Vec<String>,
    state_values: Vec<Option<f64>>,
    header_bg: String,
    finding_state: Option<usize>,
    has_finding: bool,
    beliefs: Option<Vec<f64>>,
    decision_eu: Option<Vec<Option<f64>>>,
    utility_ev: Option<f64>,
}

fn node_render_data(doc: &Document, bridge: &EngineBridge, id: NodeId) -> NodeRender {
    let n = doc.net.node(id);
    let v = doc.visual.get(id);
    let finding = doc.evidence.get(id);
    let kind = n.kind;
    NodeRender {
        title: if n.title.is_empty() { n.name.clone() } else { n.title.clone() },
        kind,
        display: v.map(|v| v.display).unwrap_or_default(),
        state_names: n.states.iter().map(|st| st.name.clone()).collect(),
        state_values: n.states.iter().map(|st| st.value).collect(),
        header_bg: v
            .and_then(|v| v.color)
            .map(|c| format!("rgb({},{},{})", c[0], c[1], c[2]))
            .unwrap_or_else(|| kind_header_color(kind).to_string()),
        finding_state: match finding {
            Some(Finding::Hard(st)) => Some(*st),
            _ => None,
        },
        has_finding: finding.is_some(),
        beliefs: bridge.beliefs.get(id).cloned(),
        decision_eu: bridge.decision_eu.get(id).cloned(),
        utility_ev: bridge.utility_ev.get(id).copied().filter(|v| v.is_finite()),
    }
}

fn write_node(s: &mut String, doc: &Document, bridge: &EngineBridge, id: NodeId, r: Rect) {
    let d = node_render_data(doc, bridge, id);
    // Same as the canvas minus UI-only borders (selection, link target).
    let (border_color, border_width) =
        if d.has_finding { ("rgb(90,90,90)", 2.5) } else { ("rgb(140,140,140)", 1.2) };

    let _ = writeln!(
        s,
        "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.0}\" height=\"{:.0}\" rx=\"5\" \
         fill=\"{}\" stroke=\"{border_color}\" stroke-width=\"{border_width}\"/>",
        r.x,
        r.y,
        r.w,
        r.h,
        kind_body_color(d.kind),
    );

    // U+25CF instead of the canvas's U+23FA: plain text-font glyph, so it
    // survives rasterization (resvg cannot draw color-emoji fonts).
    let title = format!("{}{}", d.title, if d.has_finding { " ●" } else { "" });
    let show_bars = d.display == DisplayMode::BeliefBars && d.kind != NodeKind::Utility;
    let title_only = d.display == DisplayMode::TitleOnly && d.kind != NodeKind::Utility;

    if show_bars {
        write_header(s, &d.header_bg, &title, r);
        for i in 0..d.state_names.len() {
            write_belief_row(s, &d, i, r);
        }
    } else if title_only {
        let _ = writeln!(
            s,
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"12\" \
             font-weight=\"500\" fill=\"{TEXT_DARK}\">{}</text>",
            r.x + r.w / 2.0,
            r.y + r.h / 2.0 + 0.35 * 12.0,
            esc(&fit(&title, r.w - 16.0, 12.0)),
        );
    } else {
        // ExpectedValue display (and utility nodes always): title on top,
        // EV/EU value at the bottom — same computation as the canvas.
        let ev_text = if d.kind == NodeKind::Utility {
            match d.utility_ev {
                Some(v) => format!("EU = {v:.2}"),
                None => "EU = --".into(),
            }
        } else {
            let states: Vec<bn_core::model::State> = d
                .state_names
                .iter()
                .zip(&d.state_values)
                .map(|(n, v)| bn_core::model::State { name: n.clone(), value: *v })
                .collect();
            match expected_value(&states, d.beliefs.as_deref()) {
                Some(v) => format!("E = {v:.3}"),
                None => "E = --".into(),
            }
        };
        let cx = r.x + r.w / 2.0;
        let _ = write!(
            s,
            "<text x=\"{cx:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"12\" \
             font-weight=\"500\" fill=\"{TEXT_DARK}\">{}</text>\n\
             <text x=\"{cx:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"11\" \
             font-family=\"{FONT_MONO}\" fill=\"{TEXT_MONO}\">{}</text>\n",
            r.y + 12.0 + 0.35 * 12.0,
            esc(&fit(&title, r.w - 16.0, 12.0)),
            r.y + r.h - 12.0 + 0.35 * 11.0,
            esc(&ev_text),
        );
    }
}

/// 20px header band with rounded top corners (radius 4, inside the node's 5).
fn write_header(s: &mut String, bg: &str, title: &str, r: Rect) {
    let (x0, x1, y) = (r.x, r.x + r.w, r.y);
    let _ = write!(
        s,
        "<path d=\"M {sx:.1} {y:.1} H {ex:.1} A 4 4 0 0 1 {x1:.1} {ry:.1} V {by:.1} H {x0:.1} \
         V {ry:.1} A 4 4 0 0 1 {sx:.1} {y:.1} Z\" fill=\"{bg}\"/>\n\
         <text x=\"{tx:.1}\" y=\"{ty:.1}\" font-size=\"12\" font-weight=\"500\" \
         fill=\"{TEXT_DARK}\">{title}</text>\n",
        sx = x0 + 4.0,
        ex = x1 - 4.0,
        ry = y + 4.0,
        by = y + 20.0,
        tx = x0 + 6.0,
        ty = y + 10.0 + 0.35 * 12.0,
        title = esc(&fit(title, r.w - 12.0, 12.0)),
    );
}

/// One state row, matching the canvas grid `47px 40px 1fr` with 4px gaps and
/// 5px side padding: name | right-aligned value | bar track with fill.
fn write_belief_row(s: &mut String, d: &NodeRender, i: usize, r: Rect) {
    let (val_text, frac, bar_color) = belief_row_display(
        d.kind,
        i,
        d.beliefs.as_deref(),
        d.decision_eu.as_deref(),
        d.finding_state == Some(i),
    );
    let top = r.y + 22.0 + i as f64 * 16.0;
    let mid = top + 8.0;
    let track_x = r.x + 100.0;
    let track_w = r.w - 105.0;
    let _ = write!(
        s,
        "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"10.5\" fill=\"{TEXT_MONO}\">{}</text>\n\
         <text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"10\" \
         font-family=\"{FONT_MONO}\" xml:space=\"preserve\" fill=\"{TEXT_MONO}\">{}</text>\n\
         <rect x=\"{track_x:.1}\" y=\"{:.1}\" width=\"{track_w:.1}\" height=\"10\" \
         fill=\"{TRACK_BG}\" stroke=\"{TRACK_BORDER}\" stroke-width=\"1\"/>\n",
        r.x + 5.0,
        mid + 0.35 * 10.5,
        esc(&fit(&d.state_names[i], 47.0, 10.5)),
        r.x + 96.0,
        mid + 0.35 * 10.0,
        esc(&val_text),
        top + 3.0,
    );
    if frac > 0.0 {
        let _ = writeln!(
            s,
            "<rect x=\"{track_x:.1}\" y=\"{:.1}\" width=\"{:.2}\" height=\"10\" \
             fill=\"{bar_color}\"/>",
            top + 3.0,
            frac.clamp(0.0, 1.0) * track_w,
        );
    }
}

/// XML-escape text and attribute content.
fn esc(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Approximate the canvas's CSS `truncate` with a character budget
/// (~0.55em average glyph width; `node_size` assumes 7.5px/char at 12px, so
/// this only kicks in for headers and long state names).
fn fit(text: &str, max_px: f64, font_px: f64) -> String {
    let budget = (max_px / (font_px * 0.55)).floor().max(1.0) as usize;
    if text.chars().count() <= budget {
        return text.to_string();
    }
    let mut out: String = text.chars().take(budget.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use bn_core::model::NodeKind;
    use bn_session::{Document, EngineBridge, Point, Session};

    use super::*;
    use crate::canvas::node::{BAR_FINDING, BAR_ORANGE};

    #[test]
    fn empty_network_exports_nothing() {
        assert!(network_svg(&Document::new(), &EngineBridge::default()).is_none());
    }

    #[test]
    fn structure_styles_and_size() {
        let mut doc = Document::new();
        let a = doc.add_node_at(NodeKind::Chance, Point::new(10.0, 20.0)).unwrap();
        let b = doc.add_node_at(NodeKind::Decision, Point::new(300.0, 20.0)).unwrap();
        let u = doc.add_node_at(NodeKind::Utility, Point::new(600.0, 20.0)).unwrap();
        doc.begin_change();
        doc.net.add_edge(a, b).unwrap();
        doc.net.add_edge(b, u).unwrap();

        let svg = network_svg(&doc, &EngineBridge::default()).unwrap();
        assert!(svg.starts_with("<svg "));
        // Bounds: x 10..750, y 20..76 → 740x56 plus 24px padding per side.
        assert!(svg.contains("width=\"788\" height=\"104\""));
        // Exactly one informational (a→b, child is a decision) dashed edge.
        assert_eq!(svg.matches("stroke-dasharray=\"8 5\"").count(), 1);
        assert_eq!(svg.matches("marker-end=\"url(#bn-arrow)\"").count(), 2);
        for body in ["rgb(255,248,220)", "rgb(219,233,255)", "rgb(255,226,226)"] {
            assert!(svg.contains(body), "missing body color {body}");
        }
        // Uncompiled bridge → placeholder values, no bars.
        assert!(svg.contains(">--<"));
        assert!(!svg.contains(BAR_ORANGE));
    }

    #[test]
    fn beliefs_render_after_recompute() {
        let mut s = Session::default();
        s.doc.add_node_at(NodeKind::Chance, Point::new(0.0, 0.0)).unwrap();
        bn_session::ops::edit::recompute(&mut s);
        let svg = network_svg(&s.doc, &s.bridge).unwrap();
        assert!(svg.contains(" 50.0"), "default 2-state uniform belief");
        assert!(svg.contains(BAR_ORANGE));
    }

    #[test]
    fn finding_gets_marker_bar_color_and_border() {
        let mut s = Session::default();
        let a = s.doc.add_node_at(NodeKind::Chance, Point::new(0.0, 0.0)).unwrap();
        bn_session::ops::evidence::toggle_finding(&mut s, a, 0).unwrap();
        let svg = network_svg(&s.doc, &s.bridge).unwrap();
        assert!(svg.contains("●"), "evidence marker");
        assert!(svg.contains(BAR_FINDING));
        assert!(svg.contains("rgb(90,90,90)"));
    }

    #[test]
    fn notes_export_on_top_with_wrapped_escaped_text() {
        let mut doc = Document::new();
        doc.add_node_at(NodeKind::Chance, Point::new(0.0, 0.0)).unwrap();
        let id = doc.add_note_at(Point::new(400.0, 0.0));
        doc.notes[id].text = "hello <world>\nsecond".into();
        doc.notes[id].color = [181, 220, 255];

        let svg = network_svg(&doc, &EngineBridge::default()).unwrap();
        assert!(svg.contains("rgb(181,220,255)"), "note body color");
        assert!(svg.contains("hello &lt;world&gt;"));
        assert!(svg.contains("second"), "newline becomes its own line");
        // Notes are written after every node → they render on top.
        let note_pos = svg.find("rgb(181,220,255)").unwrap();
        let node_pos = svg.find("rgb(255,248,220)").unwrap();
        assert!(note_pos > node_pos);

        // A notes-only document is exportable.
        let mut only = Document::new();
        only.add_note_at(Point::new(0.0, 0.0));
        assert!(network_svg(&only, &EngineBridge::default()).is_some());
    }

    #[test]
    fn note_markdown_exports_as_plain_text() {
        let mut doc = Document::new();
        let id = doc.add_note_at(Point::new(0.0, 0.0));
        doc.notes[id].text = "# Head\n\n* item one\n* item two".into();

        let svg = network_svg(&doc, &EngineBridge::default()).unwrap();
        assert!(svg.contains(">Head</text>"), "heading text without the # marker");
        assert!(svg.contains(">item one</text>"), "list text without the * marker");
        assert!(!svg.contains("# Head"));
        assert!(!svg.contains("* item"));
    }

    #[test]
    fn collapsed_note_exports_bar_only() {
        let mut doc = Document::new();
        let id = doc.add_note_at(Point::new(0.0, 0.0));
        doc.notes[id].text = "headline\nhidden body".into();
        doc.notes[id].collapsed = true;

        let svg = network_svg(&doc, &EngineBridge::default()).unwrap();
        assert!(svg.contains("headline"));
        assert!(!svg.contains("hidden body"), "collapsed body text must not render");
        // Only the darker bar fill appears, not the body fill.
        assert!(svg.contains(&super::note_bar_color(doc.notes[id].color)));
        assert!(!svg.contains(&super::note_fill(doc.notes[id].color)));
    }

    #[test]
    fn note_text_wraps_greedily() {
        // budget = 100 / (10 * 0.55) = 18 chars
        let lines = wrap_note_text("aaa bbb ccc ddd eee fff", 100.0, 10.0);
        assert_eq!(lines, vec!["aaa bbb ccc ddd", "eee fff"]);
        // Over-budget single word is hard-split.
        let lines = wrap_note_text("abcdefghijklmnopqrstuvwxyz", 100.0, 10.0);
        assert_eq!(lines, vec!["abcdefghijklmnopqr", "stuvwxyz"]);
        // Blank lines survive.
        let lines = wrap_note_text("a\n\nb", 100.0, 10.0);
        assert_eq!(lines, vec!["a", "", "b"]);
    }

    #[test]
    fn user_text_is_escaped() {
        let mut doc = Document::new();
        let a = doc.add_node_at(NodeKind::Chance, Point::new(0.0, 0.0)).unwrap();
        doc.net.set_title(a, "A<&>\"B".into());
        let svg = network_svg(&doc, &EngineBridge::default()).unwrap();
        assert!(svg.contains("A&lt;&amp;&gt;&quot;B"));
        assert!(!svg.contains("A<&>"));
    }
}
