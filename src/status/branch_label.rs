
use crate::git::GitHeadState;



pub(crate) fn branch_label(head: &GitHeadState) -> String {
    if let Some(branch) = &head.branch {
        branch.clone()
    } else if let Some(commit) = &head.commit {
        format!("detached@{}", commit.chars().take(12).collect::<String>())
    } else {
        "unborn".to_owned()
    }
}

