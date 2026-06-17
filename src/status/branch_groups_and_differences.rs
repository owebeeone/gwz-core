use std::collections::BTreeMap;



pub(crate) fn branch_groups_and_differences(
    branches: &[crate::GitMemberBranchStatus],
) -> (Vec<crate::GitBranchGroup>, Vec<crate::GitBranchDifference>) {
    let mut by_label: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for branch in branches {
        let entry = by_label
            .entry(branch.label.clone())
            .or_insert_with(|| (Vec::new(), Vec::new()));
        entry.0.push(branch.member_id.clone());
        entry.1.push(branch.member_path.clone());
    }

    let groups = by_label
        .iter()
        .map(
            |(label, (member_ids, member_paths))| crate::GitBranchGroup {
                label: label.clone(),
                member_ids: member_ids.clone(),
                member_paths: member_paths.clone(),
            },
        )
        .collect::<Vec<_>>();
    let Some(majority) = groups.iter().max_by_key(|group| {
        (
            group.member_ids.len(),
            std::cmp::Reverse(group.label.clone()),
        )
    }) else {
        return (groups, Vec::new());
    };
    if groups.len() <= 1 {
        return (groups, Vec::new());
    }

    let majority_label = majority.label.clone();
    let differences = groups
        .iter()
        .filter(|group| group.label != majority_label)
        .map(|group| crate::GitBranchDifference {
            label: group.label.clone(),
            majority_label: Some(majority_label.clone()),
            member_ids: group.member_ids.clone(),
            member_paths: group.member_paths.clone(),
            message: Some(format!(
                "{} differs from majority branch {}",
                group.member_paths.join(", "),
                majority_label
            )),
        })
        .collect();

    (groups, differences)
}

