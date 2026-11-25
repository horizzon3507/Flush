//! Port of `src/functions/builtins/canvas.ts` — the canvas provider is
//! deferred, so every builtin reports NATIVE_UNAVAILABLE.

use crate::errors::{codes, FnError};
use crate::types::Definitions;

macro_rules! unavailable {
    ($d:ident, $(($name:expr, $min:expr, $max:expr, $usage:expr, $desc:expr)),+ $(,)?) => {$(
        $d.add($name, "canvas", $min, $max, $usage, $desc, |c| {
            Box::pin(async move {
                let _ = c;
                Err(FnError::new(codes::NATIVE_UNAVAILABLE, "Canvas is not available.").into())
            })
        });
    )+};
}

pub fn register(d: &mut Definitions) {
    unavailable!(
        d,
        (
            "canvasCreate",
            3,
            4,
            "[id;largura;altura;opções?]",
            "Creates a composition canvas private to the current execution."
        ),
        (
            "canvasFont",
            3,
            4,
            "[id;família;fonte;opções?]",
            "Registers a TTF/OTF font on the scene without exposing its bytes to variables."
        ),
        (
            "canvasRect",
            5,
            6,
            "[id;x;y;largura;altura;opções?]",
            "Draws a filled or stroked rectangle."
        ),
        (
            "canvasCircle",
            4,
            5,
            "[id;x;y;raio;opções?]",
            "Draws a circle by center and radius."
        ),
        (
            "canvasLine",
            5,
            6,
            "[id;x1;y1;x2;y2;opções?]",
            "Draws a line with optional width and ending."
        ),
        (
            "canvasImage",
            6,
            7,
            "[id;fonte;x;y;largura;altura;opções?]",
            "Draws a configured local image, bytes or allowed URL."
        ),
        (
            "canvasAvatar",
            5,
            6,
            "[id;fonte;x;y;tamanho;opções?]",
            "Draws a square image with a circular clip and optional border."
        ),
        (
            "canvasText",
            4,
            5,
            "[id;texto;x;y;opções?]",
            "Draws text with grapheme-aware fit, wrap and overflow."
        ),
        (
            "canvasBadges",
            5,
            6,
            "[id;itens;x;y;larguraMáxima;opções?]",
            "Composes measured badges and reserves room for the counter."
        ),
        (
            "canvasProgress",
            7,
            8,
            "[id;x;y;largura;altura;valor;máximo;opções?]",
            "Draws progress clamped to zero and the maximum."
        ),
        (
            "canvasSend",
            1,
            2,
            "[id;opções?]",
            "Renders and sends an image without an extra automatic reply."
        ),
        (
            "canvasDispose",
            1,
            1,
            "[id]",
            "Disposes the execution's canvas to free its slot and reuse the ID."
        ),
    );
}
