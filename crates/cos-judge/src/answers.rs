//! Typed answers. jev answers each question with one of three primitives:
//!
//! - **yes/no** (`noul`): a probability that a statement is true, with no
//!   confidence field ([`YesNo`]);
//! - **choice**: the picked option, every option's probability and a
//!   confidence ([`Choice`]), decoded straight into a Rust enum;
//! - **score**: a position on an ordered scale, its probabilities and a
//!   confidence ([`Score`]).
//!
//! An option jev returns that the enum doesn't have is an error
//! ([`JudgeError::UnknownOption`]), never a silent fallback.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::JudgeError;

/// A yes/no answer: how likely the statement is true, from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct YesNo {
    pub probability: f64,
}

/// A choice answer decoded into `T`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice<T> {
    pub picked: T,
    /// Every option's probability, by its name in the questions file.
    pub probabilities: BTreeMap<String, f64>,
    /// How peaked the distribution is, from 0 to 1. It describes jev's
    /// certainty, not its accuracy.
    pub confidence: f64,
}

/// A score answer: the probability-weighted mean of the level indices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Score {
    pub value: f64,
    pub probabilities: BTreeMap<String, f64>,
    pub confidence: f64,
}

/// A Rust enum jev's choice answers decode into. Implemented with
/// [`choice_options!`](crate::choice_options).
pub trait ChoiceOption: Sized + Copy + 'static {
    /// Every option, by its name in the questions file.
    const OPTIONS: &'static [(&'static str, Self)];

    fn from_option(name: &str) -> Option<Self> {
        Self::OPTIONS
            .iter()
            .find(|(option, _)| *option == name)
            .map(|(_, value)| *value)
    }

    fn as_option(self) -> &'static str;
}

/// Declares an enum whose variants are a choice question's options, with
/// [`ChoiceOption`] and serde using the option names.
#[macro_export]
macro_rules! choice_options {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($(#[$vmeta:meta])* $variant:ident = $option:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $crate::ChoiceOption for $name {
            const OPTIONS: &'static [(&'static str, Self)] = &[$(($option, Self::$variant)),+];

            fn as_option(self) -> &'static str {
                match self {
                    $(Self::$variant => $option),+
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str($crate::ChoiceOption::as_option(*self))
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str($crate::ChoiceOption::as_option(*self))
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let name = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                <Self as $crate::ChoiceOption>::from_option(&name)
                    .ok_or_else(|| ::serde::de::Error::custom(format!("unknown option `{name}`")))
            }
        }
    };
}

/// The raw `answers` object of a response, keyed by question.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawAnswers {
    pub answers: serde_json::Map<String, Value>,
}

impl RawAnswers {
    fn get(&self, question: &str, kind: &str) -> Result<&Value, JudgeError> {
        let answer = self
            .answers
            .get(question)
            .ok_or_else(|| JudgeError::Missing(question.to_owned()))?;
        match answer.get("type").and_then(Value::as_str) {
            Some(found) if found == kind => Ok(answer),
            found => Err(JudgeError::Decode(format!(
                "`{question}` came back as {}, not {kind}",
                found.unwrap_or("an answer without a type")
            ))),
        }
    }

    pub fn yes_no(&self, question: &str) -> Result<YesNo, JudgeError> {
        let answer = self.get(question, "noul")?;
        let probability = number(answer, "noul", question)?;
        Ok(YesNo { probability })
    }

    pub fn choice<T: ChoiceOption>(&self, question: &str) -> Result<Choice<T>, JudgeError> {
        let answer = self.get(question, "choice")?;
        let name = answer
            .get("choice")
            .and_then(Value::as_str)
            .ok_or_else(|| JudgeError::Decode(format!("`{question}` has no `choice`")))?;
        let picked = T::from_option(name).ok_or_else(|| JudgeError::UnknownOption {
            question: question.to_owned(),
            option: name.to_owned(),
        })?;
        Ok(Choice {
            picked,
            probabilities: probabilities(answer, question)?,
            confidence: number(answer, "confidence", question)?,
        })
    }

    pub fn score(&self, question: &str) -> Result<Score, JudgeError> {
        let answer = self.get(question, "score")?;
        Ok(Score {
            value: number(answer, "score", question)?,
            probabilities: probabilities(answer, question)?,
            confidence: number(answer, "confidence", question)?,
        })
    }
}

fn number(answer: &Value, field: &str, question: &str) -> Result<f64, JudgeError> {
    answer
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| JudgeError::Decode(format!("`{question}` has no number `{field}`")))
}

fn probabilities(answer: &Value, question: &str) -> Result<BTreeMap<String, f64>, JudgeError> {
    let Some(map) = answer.get("probabilities").and_then(Value::as_object) else {
        return Ok(BTreeMap::new());
    };
    map.iter()
        .map(|(option, probability)| {
            probability
                .as_f64()
                .map(|probability| (option.clone(), probability))
                .ok_or_else(|| {
                    JudgeError::Decode(format!("`{question}` has a probability that isn't a number"))
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    choice_options! {
        enum Colour { Red = "red", Blue = "blue" }
    }

    fn raw(answers: Value) -> RawAnswers {
        RawAnswers {
            answers: answers.as_object().unwrap().clone(),
        }
    }

    #[test]
    fn decodes_each_primitive() {
        let answers = raw(json!({
            "urgent": { "type": "noul", "noul": 0.95 },
            "colour": { "type": "choice", "choice": "blue", "probabilities": { "red": 0.1, "blue": 0.9 }, "confidence": 0.83 },
            "anger": { "type": "score", "score": 1.04, "legend": { "0": "calm" }, "probabilities": { "0": 0.1, "1": 0.76, "2": 0.14 }, "confidence": 0.94 },
        }));

        assert_eq!(answers.yes_no("urgent").unwrap().probability, 0.95);
        let colour = answers.choice::<Colour>("colour").unwrap();
        assert_eq!(colour.picked, Colour::Blue);
        assert_eq!(colour.confidence, 0.83);
        assert_eq!(colour.probabilities["red"], 0.1);
        let anger = answers.score("anger").unwrap();
        assert_eq!((anger.value, anger.confidence), (1.04, 0.94));
    }

    #[test]
    fn an_unknown_option_is_an_error() {
        let answers = raw(json!({
            "colour": { "type": "choice", "choice": "green", "probabilities": {}, "confidence": 0.99 },
        }));

        let error = answers.choice::<Colour>("colour").unwrap_err();

        assert!(
            matches!(&error, JudgeError::UnknownOption { option, .. } if option == "green"),
            "{error:?}"
        );
    }

    #[test]
    fn missing_or_mistyped_answers_are_errors() {
        let answers = raw(json!({ "colour": { "type": "noul", "noul": 0.5 } }));

        assert!(matches!(answers.yes_no("urgent"), Err(JudgeError::Missing(_))));
        assert!(matches!(
            answers.choice::<Colour>("colour"),
            Err(JudgeError::Decode(_))
        ));
        let no_number = raw(json!({ "urgent": { "type": "noul" } }));
        assert!(matches!(no_number.yes_no("urgent"), Err(JudgeError::Decode(_))));
    }

    #[test]
    fn options_round_trip_through_serde() {
        assert_eq!(serde_json::to_value(Colour::Red).unwrap(), json!("red"));
        assert_eq!(
            serde_json::from_value::<Colour>(json!("blue")).unwrap(),
            Colour::Blue
        );
        assert!(serde_json::from_value::<Colour>(json!("green")).is_err());
    }
}
