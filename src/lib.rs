//! Speculos client written in Rust for Ledger integration testing.

#![deny(missing_docs)]

use std::{
    borrow::Cow,
    error::Error,
    fmt::Display,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize, ser::SerializeSeq};

/// Speculos client.
///
/// The Speculos process owned by [`SpeculosClient`] will be terminated upon dropping.
#[derive(Debug)]
pub struct SpeculosClient {
    process: Child,
    port: u16,
    client: Client,
}

/// Ledger device model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceModel {
    /// Ledger Nano S.
    Nanos,
    /// Ledger Nano X.
    Nanox,
    /// Ledger Nano S Plus.
    Nanosp,
    /// Ledger Blue.
    Blue,
    /// Ledger Stax.
    Stax,
    /// Ledger Flex.
    Flex,
}

/// Speculos automation rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutomationRule<'a> {
    /// Exact text match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Cow<'a, str>>,
    /// Regex text match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regexp: Option<Cow<'a, str>>,
    /// X coordinate match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<u32>,
    /// Y coordinate match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<u32>,
    /// Conditions for this rule to be activated.
    pub conditions: &'a [AutomationCondition<'a>],
    /// Actions to perform when this rule is applied.
    pub actions: &'a [AutomationAction<'a>],
}

/// Speculos automation actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationAction<'a> {
    /// Press or release a button.
    Button {
        /// The button whose pressed status is to be updated.
        button: Button,
        /// The pressed status to change to.
        pressed: bool,
    },
    /// Touch or release the screen.
    Finger {
        /// The X coordinate whose touched status is to be updated.
        x: u32,
        /// The Y coordinate whose touched status is to be updated.
        y: u32,
        /// The touched status to change to.
        touched: bool,
    },
    /// Set a variable to a boolean value.
    Setbool {
        /// Name of the variable to be updated.
        varname: Cow<'a, str>,
        /// The new variable value.
        value: bool,
    },
    /// Exit speculos.
    Exit,
}

/// Condition for Speculos automation rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationCondition<'a> {
    /// Name of the variable to be updated.
    pub varname: Cow<'a, str>,
    /// The new variable value.
    pub value: bool,
}

/// Ledger buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// The left button.
    Left,
    /// The right button.
    Right,
}

/// Speculos client errors.
#[derive(Debug)]
pub enum SpeculosError {
    /// System IO errors.
    IoError(std::io::Error),
    /// HTTP errors from `reqwest.
    ReqwestError(reqwest::Error),
}

#[derive(Serialize)]
struct PostApduRequest<'a> {
    #[serde(with = "hex")]
    data: &'a [u8],
}

#[derive(Deserialize)]
struct PostApduResponse {
    #[serde(with = "hex")]
    data: Vec<u8>,
}

#[derive(Serialize)]
struct PostAutomationRequest<'a> {
    version: u32,
    rules: &'a [AutomationRule<'a>],
}

impl SpeculosClient {
    /// Creates a new [`SpeculosClient`] by launching the `speculos` command.
    ///
    /// This method requires the `speculos` command to be available from `PATH`.
    ///
    /// Use different `port` values when launching multiple instances to avoid port conflicts.
    pub fn new<P: AsRef<Path>>(
        model: DeviceModel,
        port: u16,
        app: P,
    ) -> Result<Self, SpeculosError> {
        let mut process = Command::new("speculos")
            .args([
                "--api-port",
                &port.to_string(),
                "--apdu-port",
                "0",
                "-m",
                model.slug(),
                "--display",
                "headless",
                &app.as_ref().display().to_string(),
            ])
            .stderr(Stdio::piped())
            .spawn()?;

        // Wait for process to be ready by monitoring stderr
        if let Some(stderr) = process.stderr.take() {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if line.contains("launcher: using default app name & version") {
                    break;
                }
            }
        }

        Ok(Self {
            process,
            port,
            client: ClientBuilder::new()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        })
    }

    /// Sends an APDU command via the API.
    ///
    /// This method accepts and returns raw bytes. The caller should handle parsing.
    ///
    /// A common choice is to use `APDUCommand` and `APDUAnswer` types from the `coins-ledger`
    /// crate.
    pub async fn apdu(&self, data: &[u8]) -> Result<Vec<u8>, SpeculosError> {
        let response = self
            .client
            .post(format!("http://localhost:{}/apdu", self.port))
            .json(&PostApduRequest { data })
            .send()
            .await?;
        let body = response.json::<PostApduResponse>().await.unwrap();

        Ok(body.data)
    }

    /// Sends an automation request via the API.
    pub async fn automation(&self, rules: &[AutomationRule<'_>]) -> Result<(), SpeculosError> {
        let response = self
            .client
            .post(format!("http://localhost:{}/automation", self.port))
            .json(&PostAutomationRequest { version: 1, rules })
            .send()
            .await?;

        response.error_for_status()?;
        Ok(())
    }
}

impl Drop for SpeculosClient {
    fn drop(&mut self) {
        let _ = self.process.kill();
    }
}

impl DeviceModel {
    /// Gets the model slug to be used on Speculos.
    pub const fn slug(&self) -> &'static str {
        match self {
            Self::Nanos => "nanos",
            Self::Nanox => "nanox",
            Self::Nanosp => "nanosp",
            Self::Blue => "blue",
            Self::Stax => "stax",
            Self::Flex => "flex",
        }
    }
}

impl<'a> Serialize for AutomationCondition<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.varname)?;
        seq.serialize_element(&self.value)?;
        seq.end()
    }
}

impl<'a> Serialize for AutomationAction<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Button { button, pressed } => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("button")?;
                seq.serialize_element(&match button {
                    Button::Left => 1,
                    Button::Right => 2,
                })?;
                seq.serialize_element(pressed)?;
                seq.end()
            }
            Self::Finger { x, y, touched } => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("finger")?;
                seq.serialize_element(&x)?;
                seq.serialize_element(&y)?;
                seq.serialize_element(touched)?;
                seq.end()
            }
            Self::Setbool { varname, value } => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("setbool")?;
                seq.serialize_element(varname)?;
                seq.serialize_element(value)?;
                seq.end()
            }
            Self::Exit => {
                let mut seq = serializer.serialize_seq(Some(1))?;
                seq.serialize_element("exit")?;
                seq.end()
            }
        }
    }
}

impl From<std::io::Error> for SpeculosError {
    fn from(value: std::io::Error) -> Self {
        Self::IoError(value)
    }
}

impl From<reqwest::Error> for SpeculosError {
    fn from(value: reqwest::Error) -> Self {
        Self::ReqwestError(value)
    }
}

impl Display for SpeculosError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(error) => write!(f, "{}", error),
            Self::ReqwestError(error) => write!(f, "{}", error),
        }
    }
}

impl Error for SpeculosError {}
