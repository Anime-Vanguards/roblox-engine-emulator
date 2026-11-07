mod axes;
mod brick_color;
mod cframe;
mod color3;
mod color_sequence;
mod color_sequence_keypoint;
mod content;
mod r#enum;
mod r#enum_item;
mod r#enums;
mod faces;
mod font;
mod number_range;
mod number_sequence;
mod number_sequence_keypoint;
mod physical_properties;
mod ray;
mod rect;
mod region3;
mod region3int16;
mod udim;
mod udim2;
mod unique_id;
mod vector2;
mod vector2int16;
mod vector3;
mod vector3int16;

pub use axes::Axes;
pub use brick_color::BrickColor;
pub use cframe::CFrame;
pub use color_sequence::ColorSequence;
pub use color_sequence_keypoint::ColorSequenceKeypoint;
pub use color3::Color3;
pub use content::Content;
pub use r#enum::Enum;
pub use r#enum_item::EnumItem;
pub use r#enums::Enums;
pub use faces::Faces;
pub use font::Font;
pub use number_range::NumberRange;
pub use number_sequence::NumberSequence;
pub use number_sequence_keypoint::NumberSequenceKeypoint;
pub use physical_properties::PhysicalProperties;
pub use ray::Ray;
pub use rect::Rect;
pub use region3::Region3;
pub use region3int16::Region3int16;
pub use udim::UDim;
pub use udim2::UDim2;
pub use unique_id::UniqueId;
pub use vector2::Vector2;
pub use vector2int16::Vector2int16;
pub use vector3::{Vector3, VectorArg, charge_host_calls, charge_host_nanos, datatype_price, native_vector};
pub use vector3int16::Vector3int16;

/// A number as C's `printf("%.*g", precision, value)` writes it, the way Roblox's datatypes print their components in
/// `tostring`: `precision` significant digits rounded from the exact value, trailing zeros dropped, the exponent form
/// (`1.00000002e+20`, `1e-07`, two exponent digits at least) when the exponent is below -4 or at least `precision`,
/// and `-0` for negative zero. Measured in a playing client (Studio 0.744): CFrame, Vector2, UDim, Rect, Ray and
/// PhysicalProperties print 9 digits (`0.333333343`), Color3, NumberRange, the sequences and TweenInfo's times 6
/// (`0.333333`). Not-a-number and infinities print as Luau's tostring does.
pub fn format_g(value: f64, precision: usize) -> String {
    let precision = precision.max(1);
    if value.is_nan() {
        return if value.is_sign_negative() { "-nan".into() } else { "nan".into() };
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf".into() } else { "inf".into() };
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    // the exponent of the value rounded to `precision` digits decides the style, as C's %g does
    let scientific = format!("{:.*e}", precision - 1, value);
    let (mantissa, exponent) = scientific.split_once('e').expect("an exponent");
    let exponent: i32 = exponent.parse().expect("a decimal exponent");
    if exponent < -4 || exponent >= precision as i32 {
        let mantissa = trim_fraction(mantissa);
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs());
    }
    let decimals = (precision as i32 - 1 - exponent).max(0) as usize;
    trim_fraction(&format!("{value:.decimals$}")).to_string()
}

fn trim_fraction(text: &str) -> &str {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { text }
}
