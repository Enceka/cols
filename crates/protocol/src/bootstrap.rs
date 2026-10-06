//! Static evidence: client RVA 0x241efc..0x241f85 reads five whitespace-delimited
//! fields using %59s, %hu, %59s, %hu, %x. See docs/BOOTSTRAP_FINDINGS.md.
//! Endpoint B is the observed HTTPS AUTH endpoint. The final integer maps to
//! title_id (confirmed by differential captures); valid deployment values remain unknown.

use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DwauthConfig {
    pub endpoint_a: Endpoint,
    pub endpoint_b: Endpoint,
    pub title_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for ConfigError {}

impl Endpoint {
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Stricter than scanf: never generate truncated, non-ASCII or zero-port
        // input. This producer is not an emulation of malformed C scanf input.
        if self.host.is_empty()
            || self.host.len() > 59
            || !self.host.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(ConfigError("host must be 1..59 visible ASCII bytes".into()));
        }
        if self.port == 0 {
            return Err(ConfigError("client endpoint port cannot be zero".into()));
        }
        Ok(())
    }
}

impl DwauthConfig {
    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        let fields: Vec<_> = input.split_ascii_whitespace().collect();
        if fields.len() != 5 {
            return Err(ConfigError("expected exactly five fields".into()));
        }
        let endpoint = |host: &str, port: &str| -> Result<Endpoint, ConfigError> {
            let value = Endpoint {
                host: host.into(),
                port: port
                    .parse()
                    .map_err(|_| ConfigError("invalid u16 port".into()))?,
            };
            value.validate()?;
            Ok(value)
        };
        let hex = fields[4]
            .strip_prefix("0x")
            .or_else(|| fields[4].strip_prefix("0X"))
            .unwrap_or(fields[4]);
        if hex.is_empty() || hex.len() > 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ConfigError(
                "title ID must be a u32 hexadecimal integer".into(),
            ));
        }
        Ok(Self {
            endpoint_a: endpoint(fields[0], fields[1])?,
            endpoint_b: endpoint(fields[2], fields[3])?,
            title_id: u32::from_str_radix(hex, 16)
                .map_err(|_| ConfigError("invalid title ID".into()))?,
        })
    }

    pub fn render(&self) -> Result<String, ConfigError> {
        self.endpoint_a.validate()?;
        self.endpoint_b.validate()?;
        Ok(format!(
            "{} {}\n{} {}\n0x{:08X}\n",
            self.endpoint_a.host,
            self.endpoint_a.port,
            self.endpoint_b.host,
            self.endpoint_b.port,
            self.title_id
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Laboratory data; NOT a capture or an authenticated game configuration.
    const LAB: &str = "127.0.0.1 39001\n127.0.0.1 39002\n0x1234\n";

    #[test]
    fn reads_the_five_observed_scanf_fields() {
        let parsed = DwauthConfig::parse(LAB).unwrap();
        assert_eq!(parsed.endpoint_a.port, 39001);
        assert_eq!(parsed.endpoint_b.port, 39002);
        assert_eq!(parsed.title_id, 0x1234);
        assert_eq!(
            parsed.render().unwrap(),
            LAB.replace("0x1234", "0x00001234")
        );
        assert_eq!(
            DwauthConfig::parse(&LAB.replace('\n', "\r\n")).unwrap(),
            parsed
        );
    }

    #[test]
    fn rejects_ambiguous_or_truncated_configuration() {
        for bad in [
            LAB.replace("39001", "65536"),
            LAB.replace("39001", "0"),
            LAB.replace("0x1234", "100000000"),
            LAB.replace("0x1234", "-1"),
            LAB.replace("127.0.0.1", &"a".repeat(60)),
            LAB.replace("127.0.0.1", "host\0name"),
            LAB.replace("127.0.0.1", "主机"),
            format!("{LAB}extra"),
            "127.0.0.1 39001".into(),
        ] {
            assert!(DwauthConfig::parse(&bad).is_err(), "{bad:?}");
        }
    }
}
