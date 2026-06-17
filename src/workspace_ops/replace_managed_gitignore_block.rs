use std::fs;
use std::path::Path;

use crate::artifact::ManifestMember;
use crate::model::ModelResult;
use crate::workspace::WORKSPACE_DIR;


use super::*;

pub(crate) const GITIGNORE_GWZ_BEGIN: &str = "# BEGIN GWZ managed member repositories";
pub(crate) const GITIGNORE_GWZ_END: &str = "# END GWZ managed member repositories";

pub(crate) fn update_workspace_gitignore(root: &Path, members: &[ManifestMember]) -> ModelResult<()> {
    let managed = managed_gitignore_block(members);
    let path = root.join(".gitignore");
    let existing = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io_error(error)),
    };
    let updated = replace_managed_gitignore_block(&existing, &managed);
    if updated != existing {
        fs::write(path, updated).map_err(io_error)?;
    }
    Ok(())
}

pub(crate) fn managed_gitignore_block(members: &[ManifestMember]) -> String {
    let mut paths = members
        .iter()
        .map(|member| member.path.as_str())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths.dedup();

    let mut lines = vec![
        GITIGNORE_GWZ_BEGIN.to_owned(),
        format!("/{WORKSPACE_DIR}/.tmp/"),
    ];
    lines.extend(paths.into_iter().map(|path| format!("/{path}/")));
    lines.push(GITIGNORE_GWZ_END.to_owned());
    lines.push(String::new());
    lines.join("\n")
}

pub(crate) fn replace_managed_gitignore_block(existing: &str, managed: &str) -> String {
    if let Some(begin) = existing.find(GITIGNORE_GWZ_BEGIN)
        && let Some(relative_end) = existing[begin..].find(GITIGNORE_GWZ_END)
    {
        let end = begin + relative_end + GITIGNORE_GWZ_END.len();
        let after_end = if existing[end..].starts_with("\r\n") {
            end + 2
        } else if existing[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        let mut out = String::with_capacity(existing.len() + managed.len());
        out.push_str(&existing[..begin]);
        out.push_str(managed);
        out.push_str(&existing[after_end..]);
        return out;
    }

    if existing.trim().is_empty() {
        managed.to_owned()
    } else if existing.ends_with('\n') {
        format!("{existing}\n{managed}")
    } else {
        format!("{existing}\n\n{managed}")
    }
}

