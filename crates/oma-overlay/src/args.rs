//! Command line of `oma-overlay.exe`: the app passes `--pipe <name>` and
//! nothing else.

use oma_ipc::overlay::OVERLAY_PIPE_PREFIX;

/// Longest accepted pipe name, in bytes.
pub const MAX_PIPE_NAME_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Full pipe path, starting with [`OVERLAY_PIPE_PREFIX`].
    pub pipe: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgsError {
    /// `--pipe` was not given.
    MissingPipe,
    /// `--pipe` was the last argument.
    MissingValue,
    /// `--pipe` was given more than once.
    DuplicatePipe,
    /// The name lacks the overlay prefix, names nothing after it, or is too long.
    BadPipeName(String),
    /// Any other argument.
    Unknown(String),
}

impl std::fmt::Display for ArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPipe => write!(f, "--pipe <name> is required"),
            Self::MissingValue => write!(f, "--pipe needs a value"),
            Self::DuplicatePipe => write!(f, "--pipe given more than once"),
            Self::BadPipeName(name) => write!(f, "{name:?} is not an overlay pipe name"),
            Self::Unknown(arg) => write!(f, "unknown argument {arg:?}"),
        }
    }
}

impl std::error::Error for ArgsError {}

fn is_valid_pipe_name(name: &str) -> bool {
    name.len() <= MAX_PIPE_NAME_BYTES
        && name
            .strip_prefix(OVERLAY_PIPE_PREFIX)
            .is_some_and(|rest| !rest.is_empty() && !rest.contains(['\\', '/']))
}

/// Parses the arguments after the program name.
pub fn parse_args(args: &[String]) -> Result<Args, ArgsError> {
    let mut pipe = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pipe" => {
                let value = it.next().ok_or(ArgsError::MissingValue)?;
                if pipe.is_some() {
                    return Err(ArgsError::DuplicatePipe);
                }
                if !is_valid_pipe_name(value) {
                    return Err(ArgsError::BadPipeName(value.clone()));
                }
                pipe = Some(value.clone());
            }
            other => return Err(ArgsError::Unknown(other.to_owned())),
        }
    }
    pipe.map(|pipe| Args { pipe }).ok_or(ArgsError::MissingPipe)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    fn name() -> String {
        format!("{OVERLAY_PIPE_PREFIX}0f8e2c1a-3b4d-4e5f-8a6b-7c8d9e0f1a2b")
    }

    #[test]
    fn pipe_arg_required() {
        assert_eq!(parse_args(&[]), Err(ArgsError::MissingPipe));
        assert_eq!(parse_args(&v(&["--pipe"])), Err(ArgsError::MissingValue));
        assert_eq!(
            parse_args(&[String::from("--pipe"), name()]),
            Ok(Args { pipe: name() })
        );
        assert_eq!(
            parse_args(&[
                String::from("--pipe"),
                name(),
                String::from("--pipe"),
                name()
            ]),
            Err(ArgsError::DuplicatePipe)
        );
    }

    #[test]
    fn pipe_must_have_overlay_prefix() {
        for bad in [
            r"\\.\pipe\OpenMonitorAdvanced".to_owned(),
            r"\\.\pipe\Other-Overlay-1234".to_owned(),
            OVERLAY_PIPE_PREFIX.to_owned(),
            format!(r"{OVERLAY_PIPE_PREFIX}a\b"),
            format!("{OVERLAY_PIPE_PREFIX}{}", "a".repeat(MAX_PIPE_NAME_BYTES)),
        ] {
            assert_eq!(
                parse_args(&[String::from("--pipe"), bad.clone()]),
                Err(ArgsError::BadPipeName(bad))
            );
        }
        let longest = format!(
            "{OVERLAY_PIPE_PREFIX}{}",
            "a".repeat(MAX_PIPE_NAME_BYTES - OVERLAY_PIPE_PREFIX.len())
        );
        assert!(parse_args(&[String::from("--pipe"), longest]).is_ok());
    }

    #[test]
    fn unknown_args_rejected() {
        assert_eq!(
            parse_args(&[String::from("--pipe"), name(), String::from("--verbose")]),
            Err(ArgsError::Unknown("--verbose".into()))
        );
        assert_eq!(
            parse_args(&v(&["extra"])),
            Err(ArgsError::Unknown("extra".into()))
        );
        assert_eq!(
            parse_args(&[String::from("--PIPE"), name()]),
            Err(ArgsError::Unknown("--PIPE".into()))
        );
    }
}
