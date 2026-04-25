use crate::cli::SchemaArgs;
use crate::error::{CiError, Result};
use crate::schema_definitions::{all_schema, config_schema, workflow_schema};

pub fn cmd_schema(args: &SchemaArgs) -> Result<i32> {
    let schema = match args.subject.as_deref().unwrap_or("all") {
        "config" | "settings" => config_schema(),
        "workflow" | "workflows" => workflow_schema(),
        "all" => all_schema(),
        other => {
            return Err(CiError::Usage(format!(
                "unknown schema `{other}`; use `config`, `workflow`, or `all`"
            )))
        }
    };
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(0)
}
