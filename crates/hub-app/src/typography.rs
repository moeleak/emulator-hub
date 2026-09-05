//! Application aliases for the full Material 3 type scale.
use iced::widget::{Text, text::IntoFragment};
pub use material_ui_rs::text::{
    body_large, body_medium, headline_large, headline_medium, title_medium,
};
use material_ui_rs::{Theme, tokens::typography::*};

macro_rules! role {
    ($name:ident, $scale:ident) => {
        pub fn $name<'a>(content: impl IntoFragment<'a>) -> Text<'a, Theme> {
            material_ui_rs::text::type_scale(content, $scale).font(iced::Font {
                weight: if $scale.weight >= WEIGHT_MEDIUM {
                    iced::font::Weight::Medium
                } else {
                    iced::font::Weight::Normal
                },
                ..iced::Font::DEFAULT
            })
        }
    };
}
role!(headline_small, HEADLINE_SMALL);
role!(title_large, TITLE_LARGE);
role!(body_small, BODY_SMALL);
role!(label_large, LABEL_LARGE);
role!(label_medium, LABEL_MEDIUM);
