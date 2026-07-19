use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u8,
    pub target: Target,
    pub reason: Reason,
    pub evidence: Vec<Evidence>,
    pub history_complete: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Target {
    pub path: String,
    pub revision: String,
    pub line: usize,
    pub text: String,
    pub dirty: bool,
}

#[derive(Debug, Serialize)]
pub struct Reason {
    pub status: ReasonStatus,
    pub text: Option<String>,
    pub source_commit: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonStatus {
    Recorded,
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct Evidence {
    pub relation: EvidenceRelation,
    pub commit: String,
    pub authored_at: String,
    pub author_name: String,
    pub subject: String,
    pub body: String,
    pub references: Vec<Reference>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRelation {
    LastChanged,
    LineHistory,
    FileHistory,
}

#[derive(Debug, Serialize)]
pub struct Reference {
    pub number: u64,
    pub url: Option<String>,
}
