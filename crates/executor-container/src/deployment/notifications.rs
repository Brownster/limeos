//! Decode supplied *arr event bytes. No HTTP, token, endpoint or delivery authority.
use limeos_domain::deployment::notifications::{
    Input, MAX_FIELD_BYTES, Media, Mode, Normalization, normalize, valid_field,
};
use serde::{
    Deserialize, Deserializer,
    de::{IgnoredAny, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer},
};
use std::{fmt, marker::PhantomData};

pub const MAX_BODY_BYTES: usize = 256 * 1024;
pub const MAX_EPISODES: usize = 128;
pub const MAX_JSON_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    BodyTooLarge,
    InvalidBody,
    InvalidInput,
}

/// Serde's ignored-value path skips its normal recursion limit. Bound all JSON
/// containers, including skipped fields, with constant memory before decoding.
fn admitted_nesting(bytes: &[u8]) -> bool {
    let mut stack = [0; MAX_JSON_DEPTH];
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                quoted = false;
            }
        } else {
            match *byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    if depth == MAX_JSON_DEPTH {
                        return false;
                    }
                    stack[depth] = *byte;
                    depth += 1;
                }
                b']' | b'}' => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                    if !matches!((stack[depth], *byte), (b'[', b']') | (b'{', b'}')) {
                        return false;
                    }
                }
                _ => {}
            }
        }
    }
    !quoted && depth == 0
}

/// Derived struct decoders also admit positional arrays. Wire records are maps only.
struct Object<T>(T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Map<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Map<T> {
            type Value = Object<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a notification object")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(Object)
            }
        }
        de.deserialize_map(Map(PhantomData))
    }
}

struct Text(String);
impl Text {
    fn new<E: serde::de::Error>(value: &str) -> Result<Self, E> {
        if !valid_field(value) {
            return Err(E::custom("invalid notification text"));
        }
        Ok(Self(value.into()))
    }
}
impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct StringValue;
        impl Visitor<'_> for StringValue {
            type Value = Text;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "notification text of at most {MAX_FIELD_BYTES} bytes")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Text, E> {
                Text::new(value)
            }
        }
        de.deserialize_str(StringValue)
    }
}

/// Keep null as a present field, so Serde also rejects duplicate null keys.
struct Optional<T>(Option<T>);
impl<T> Default for Optional<T> {
    fn default() -> Self {
        Self(None)
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Optional<T> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(de).map(Self)
    }
}
impl Optional<Text> {
    fn text(&self) -> Option<&str> {
        self.0.as_ref().map(|t| t.0.as_str())
    }
}

/// These fields are nonnegative integral labels, not measurements or identities.
struct Number(u16);
impl<'de> Deserialize<'de> for Number {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Integer;
        impl Visitor<'_> for Integer {
            type Value = Number;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a nonnegative u16 label or decimal string")
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Number, E> {
                u16::try_from(value)
                    .map(Number)
                    .map_err(|_| E::custom("notification number limit"))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Number, E> {
                if value.is_empty() || value.len() > 5 || !value.bytes().all(|c| c.is_ascii_digit())
                {
                    return Err(E::custom("invalid notification number"));
                }
                value
                    .parse::<u16>()
                    .map(Number)
                    .map_err(|_| E::custom("notification number limit"))
            }
        }
        de.deserialize_any(Integer)
    }
}

enum TextOr<T> {
    Text(Text),
    Object(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for TextOr<T> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Value<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Value<T> {
            type Value = TextOr<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a notification string or object")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Text::new(value).map(TextOr::Text)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(TextOr::Object)
            }
        }
        de.deserialize_any(Value(PhantomData))
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Movie {
    title: Optional<Text>,
    year: Optional<Number>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Series {
    title: Optional<Text>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Episode {
    season_number: Optional<Number>,
    episode_number: Optional<Number>,
}
struct Episodes(Option<Episode>);
impl<'de> Deserialize<'de> for Episodes {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct First;
        impl<'de> Visitor<'de> for First {
            type Value = Episodes;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("at most 128 episodes")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Episodes, A::Error> {
                let Some(first) = seq.next_element::<Optional<Object<Episode>>>()? else {
                    return Ok(Episodes(None));
                };
                for _ in 1..MAX_EPISODES {
                    if seq.next_element::<IgnoredAny>()?.is_none() {
                        return Ok(Episodes(first.0.map(|object| object.0)));
                    }
                }
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("notification episode limit"));
                }
                Ok(Episodes(first.0.map(|object| object.0)))
            }
        }
        de.deserialize_seq(First)
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Named {
    name: Optional<Text>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Quality {
    quality: Optional<TextOr<Named>>,
    name: Optional<Text>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct File {
    quality: Optional<TextOr<Quality>>,
}
impl File {
    fn quality(&self) -> Option<&str> {
        let TextOr::Object(quality) = self.quality.0.as_ref()? else {
            // The frozen file-quality selector ignores direct strings.
            return None;
        };
        let value = match quality.quality.0.as_ref() {
            Some(TextOr::Object(named)) => named.name.text(),
            Some(TextOr::Text(text)) if !text.0.is_empty() => Some(text.0.as_str()),
            _ => quality.name.text(),
        };
        value.filter(|value| !value.is_empty())
    }
}
impl TextOr<Quality> {
    fn release_quality(&self) -> Option<&str> {
        let value = match self {
            Self::Text(text) => Some(text.0.as_str()),
            Self::Object(quality) => match quality.quality.0.as_ref() {
                Some(TextOr::Object(named)) => named
                    .name
                    .text()
                    .filter(|name| !name.is_empty())
                    .or_else(|| quality.name.text()),
                _ => quality.name.text(),
            },
        };
        value.filter(|value| !value.is_empty())
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Release {
    quality: Optional<TextOr<Quality>>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Payload {
    event_type: Optional<Text>,
    instance_name: Optional<Text>,
    is_upgrade: Optional<bool>,
    level: Optional<Text>,
    message: Optional<Text>,
    movie: Optional<Object<Movie>>,
    series: Optional<Object<Series>>,
    episodes: Optional<Episodes>,
    movie_file: Optional<Object<File>>,
    episode_file: Optional<Object<File>>,
    track_file: Optional<Object<File>>,
    book_file: Optional<Object<File>>,
    release: Optional<Object<Release>>,
}

/// Unknown fields are skipped, not retained. Selected malformed fields are errors,
/// including on suppressed events; no raw body/error text escapes this boundary.
pub fn decode(
    bytes: &[u8],
    mode: Mode,
    source_default: &str,
) -> Result<Normalization, DecodeError> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(DecodeError::BodyTooLarge);
    }
    if !admitted_nesting(bytes) {
        return Err(DecodeError::InvalidBody);
    }
    let Object(payload): Object<Payload> =
        serde_json::from_slice(bytes).map_err(|_| DecodeError::InvalidBody)?;
    let media = if let Some(Object(movie)) = payload.movie.0.as_ref() {
        Media::Movie {
            title: movie.title.text(),
            year: movie.year.0.as_ref().map(|n| n.0),
        }
    } else if let Some(Object(series)) = payload.series.0.as_ref() {
        let episode = payload
            .episodes
            .0
            .as_ref()
            .and_then(|episodes| episodes.0.as_ref())
            .and_then(|episode| {
                episode.season_number.0.as_ref().map(|season| {
                    (
                        season.0,
                        episode.episode_number.0.as_ref().map_or(0, |n| n.0),
                    )
                })
            });
        Media::Series {
            title: series.title.text(),
            episode,
        }
    } else {
        Media::None
    };
    let quality = [
        &payload.movie_file,
        &payload.episode_file,
        &payload.track_file,
        &payload.book_file,
    ]
    .into_iter()
    .filter_map(|file| file.0.as_ref().and_then(|object| object.0.quality()))
    .next()
    .or_else(|| {
        payload
            .release
            .0
            .as_ref()?
            .0
            .quality
            .0
            .as_ref()?
            .release_quality()
    });
    normalize(
        mode,
        source_default,
        Input {
            event_type: payload.event_type.text().unwrap_or(""),
            instance_name: payload.instance_name.text(),
            is_upgrade: payload.is_upgrade.0 == Some(true),
            media,
            quality,
            message: payload.message.text(),
            health_level: payload.level.text(),
        },
    )
    .map_err(|_| DecodeError::InvalidInput)
}

#[cfg(test)]
mod tests;
