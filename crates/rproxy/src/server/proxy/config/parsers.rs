#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use std::{ffi::OsStr, fmt::Display};

    use clap::{Arg, Command, builder::TypedValueParser, error::ErrorKind};

    // CpuRangeParser ------------------------------------------------------

    #[derive(Clone)]
    pub(crate) struct CpuRangeParser {}

    impl TypedValueParser for CpuRangeParser {
        type Value = (usize, usize);

        fn parse_ref(
            &self,
            cmd: &Command,
            arg: Option<&Arg>,
            value: &OsStr,
        ) -> Result<Self::Value, clap::Error> {
            let value = value.to_str().ok_or_else(|| clap_error(cmd, arg, "", "invalid utf-8"))?;

            let parts: Vec<_> = value.split("-").collect();

            if parts.len() > 2 {
                return Err(clap_error(cmd, arg, value, "invalid port range"));
            }

            match parts.len() {
                1 => {
                    let port = parts[0].parse::<usize>().map_err(|err| {
                        clap_error(cmd, arg, value, format!("invalid port: {err}"))
                    })?;
                    Ok((port, port))
                }

                2 => {
                    let port0 = parts[0].parse::<usize>().map_err(|err| {
                        clap_error(cmd, arg, value, format!("invalid port: {err}"))
                    })?;
                    let port1 = parts[1].parse::<usize>().map_err(|err| {
                        clap_error(cmd, arg, value, format!("invalid port: {err}"))
                    })?;
                    if port0 < port1 { Ok((port0, port1)) } else { Ok((port1, port0)) }
                }

                _ => Err(clap_error(cmd, arg, value, "invalid port range")),
            }
        }
    }

    // helpers -------------------------------------------------------------

    fn clap_error<E>(cmd: &Command, arg: Option<&Arg>, value: &str, err: E) -> clap::Error
    where
        E: Display,
    {
        match (value, arg) {
            ("", None) => clap::Error::raw(
                ErrorKind::ValueValidation,
                format!("invalid value for one of the arguments: {err}\n"),
            )
            .with_cmd(cmd),

            ("", Some(arg)) => clap::Error::raw(
                ErrorKind::ValueValidation,
                format!("invalid value for `{arg}`: {err}\n"),
            )
            .with_cmd(cmd),

            (value, None) => clap::Error::raw(
                ErrorKind::ValueValidation,
                format!("invalid value `{value}` for one of the arguments: {err}\n"),
            )
            .with_cmd(cmd),

            (value, Some(arg)) => clap::Error::raw(
                ErrorKind::ValueValidation,
                format!("invalid value `{value}` for `{arg}`: {err}\n"),
            )
            .with_cmd(cmd),
        }
    }
}
