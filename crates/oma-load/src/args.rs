//! Command line of `oma-load.exe`.

use oma_ipc::load::{KernelId, LOAD_PIPE_PREFIX};

/// Longest accepted pipe name, in bytes.
pub const MAX_PIPE_NAME_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inject {
    pub kernel: KernelId,
    pub core: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub pipe: String,
    pub inject: Option<Inject>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgsError {
    /// `--pipe` was not given.
    MissingPipe,
    /// An option lacks its value.
    MissingValue(String),
    /// An option was given more than once.
    Duplicate(String),
    /// The name lacks the load prefix, names nothing after it, or is too long.
    BadPipeName(String),
    /// `--inject-fault` was not `<kernel>[:<core>]`.
    BadInject(String),
    /// Any other argument.
    Unknown(String),
}

impl std::fmt::Display for ArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPipe => write!(f, "--pipe <name> is required"),
            Self::MissingValue(o) => write!(f, "{o} needs a value"),
            Self::Duplicate(o) => write!(f, "{o} given more than once"),
            Self::BadPipeName(n) => write!(f, "{n:?} is not a load pipe name"),
            Self::BadInject(v) => write!(f, "{v:?} is not <kernel>[:<core>]"),
            Self::Unknown(a) => write!(f, "unknown argument {a:?}"),
        }
    }
}

impl std::error::Error for ArgsError {}

fn is_valid_pipe_name(name: &str) -> bool {
    name.len() <= MAX_PIPE_NAME_BYTES
        && name
            .strip_prefix(LOAD_PIPE_PREFIX)
            .is_some_and(|rest| !rest.is_empty() && !rest.contains(['\\', '/']))
}

#[cfg(debug_assertions)]
fn parse_inject(value: &str) -> Result<Inject, ArgsError> {
    let bad = || ArgsError::BadInject(value.to_owned());
    let (kernel, core) = match value.split_once(':') {
        Some((k, c)) => (k, Some(c.parse::<u32>().map_err(|_| bad())?)),
        None => (value, None),
    };
    let kernel = match kernel {
        "k1" => KernelId::K1,
        "k2" => KernelId::K2,
        "k3" => KernelId::K3,
        "k4" => KernelId::K4,
        "k5" => KernelId::K5,
        "k7" => KernelId::K7,
        "k8" => KernelId::K8,
        "k9" => KernelId::K9,
        "k10" => KernelId::K10,
        "s1" => KernelId::S1,
        "s2" => KernelId::S2,
        "s4" => KernelId::S4,
        "s6" => KernelId::S6,
        "n3" => KernelId::N3,
        "v1" => KernelId::V1,
        "v2" => KernelId::V2,
        "v3" => KernelId::V3,
        "v4" => KernelId::V4,
        _ => return Err(bad()),
    };
    Ok(Inject { kernel, core })
}

/// Parses the arguments after the program name.
pub fn parse_args(args: &[String]) -> Result<Args, ArgsError> {
    let mut pipe = None;
    #[cfg_attr(not(debug_assertions), allow(unused_mut))]
    let mut inject = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pipe" => {
                let value = it
                    .next()
                    .ok_or_else(|| ArgsError::MissingValue(arg.clone()))?;
                if pipe.is_some() {
                    return Err(ArgsError::Duplicate(arg.clone()));
                }
                if !is_valid_pipe_name(value) {
                    return Err(ArgsError::BadPipeName(value.clone()));
                }
                pipe = Some(value.clone());
            }
            // Fault injection exists only in debug builds (DA18).
            #[cfg(debug_assertions)]
            "--inject-fault" => {
                let value = it
                    .next()
                    .ok_or_else(|| ArgsError::MissingValue(arg.clone()))?;
                if inject.is_some() {
                    return Err(ArgsError::Duplicate(arg.clone()));
                }
                inject = Some(parse_inject(value)?);
            }
            other => return Err(ArgsError::Unknown(other.to_owned())),
        }
    }
    let pipe = pipe.ok_or(ArgsError::MissingPipe)?;
    Ok(Args { pipe, inject })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::load::LOAD_PIPE_PREFIX;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    fn name() -> String {
        format!("{LOAD_PIPE_PREFIX}0f8e2c1a-3b4d-4e5f-8a6b-7c8d9e0f1a2b")
    }

    #[test]
    fn args_require_a_load_pipe_name() {
        assert_eq!(parse_args(&[]), Err(ArgsError::MissingPipe));
        assert!(parse_args(&v(&["--pipe", r"\.\pipe\OpenMonitorAdvanced-Overlay-x"])).is_err());
        assert!(parse_args(&v(&["--pipe"])).is_err());
        let ok = parse_args(&["--pipe".to_owned(), name()]).unwrap();
        assert_eq!(ok.pipe, name());
        assert_eq!(ok.inject, None);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_fault_parses_kernel_and_core() {
        let p = || "--pipe".to_owned();
        let with = |s: &str| parse_args(&[p(), name(), "--inject-fault".to_owned(), s.to_owned()]);
        assert_eq!(
            with("k2").unwrap().inject,
            Some(Inject {
                kernel: KernelId::K2,
                core: None
            })
        );
        assert_eq!(
            with("k10:3").unwrap().inject,
            Some(Inject {
                kernel: KernelId::K10,
                core: Some(3)
            })
        );
        assert!(with("k6").is_err());
        assert!(with("k1:x").is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn parse_inject_accepts_disk_kernels() {
        for (arg, kernel) in [
            ("n3", KernelId::N3),
            ("v1", KernelId::V1),
            ("v2", KernelId::V2),
            ("v3", KernelId::V3),
            ("v4", KernelId::V4),
        ] {
            let args = [
                "--pipe".to_owned(),
                name(),
                "--inject-fault".to_owned(),
                arg.to_owned(),
            ];
            assert_eq!(
                parse_args(&args).unwrap().inject,
                Some(Inject { kernel, core: None })
            );
        }
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn inject_fault_does_not_exist_in_release() {
        assert!(parse_args(&[
            "--pipe".to_owned(),
            name(),
            "--inject-fault".to_owned(),
            "k1".to_owned()
        ])
        .is_err());
    }

    #[test]
    fn unknown_argument_is_usage() {
        assert!(parse_args(&["--pipe".to_owned(), name(), "--verbose".to_owned()]).is_err());
        assert!(parse_args(&v(&["extra"])).is_err());
    }
}
