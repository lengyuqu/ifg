//! `ifg org …` — organizations and their members.

use crate::cli::{Cli, OrgCommand};
use crate::cmd::bootstrap;
use crate::error::AppError;
use crate::output;

pub async fn run(command: &OrgCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let orgs = ctx.client.orgs();
    match command {
        OrgCommand::List => {
            if cli.json || cli.template.is_some() {
                let value = orgs.list_raw().await?;
                match &cli.template {
                    Some(template) => output::print_template(
                        value.as_array().cloned().unwrap_or_default().as_slice(),
                        template,
                    ),
                    None => output::print_json(&value),
                }
                return Ok(());
            }
            let items = orgs.list().await?;
            if items.is_empty() {
                println!("No organizations. Create one with `ifg org create <name>`.");
                return Ok(());
            }
            let rows: Vec<Vec<String>> = items
                .iter()
                .map(|org| {
                    vec![
                        org.name.clone(),
                        org.display_name.clone().unwrap_or_default(),
                        org.visibility.clone().unwrap_or_else(|| "public".into()),
                        output::human_time(org.created_at.as_deref()),
                    ]
                })
                .collect();
            output::print_table(&["NAME", "DISPLAY", "VISIBILITY", "CREATED"], &rows);
            Ok(())
        }
        OrgCommand::View { name } => {
            if cli.json || cli.template.is_some() {
                let value = orgs.get_raw(name).await?;
                match &cli.template {
                    Some(template) => {
                        output::print_template(std::slice::from_ref(&value), template)
                    }
                    None => output::print_json(&value),
                }
                return Ok(());
            }
            let org = orgs.get(name).await?;
            // A failed member listing must not read as "this org has nobody":
            // show the count as unknown and say why on stderr.
            let (member_count, member_error) = match orgs.members(name).await {
                Ok(members) => (members.len().to_string(), None),
                Err(error) => ("unknown".to_string(), Some(error.to_string())),
            };
            let rows = vec![
                vec!["name".into(), org.name.clone()],
                vec![
                    "display".into(),
                    org.display_name.clone().unwrap_or_default(),
                ],
                vec![
                    "description".into(),
                    org.description.clone().unwrap_or_default(),
                ],
                vec![
                    "visibility".into(),
                    org.visibility.clone().unwrap_or_else(|| "public".into()),
                ],
                vec![
                    "owner id".into(),
                    org.owner_id.map(|id| id.to_string()).unwrap_or_default(),
                ],
                vec!["members".into(), member_count],
                vec![
                    "created".into(),
                    output::human_time(org.created_at.as_deref()),
                ],
            ];
            output::print_table(&["FIELD", "VALUE"], &rows);
            if let Some(error) = member_error {
                eprintln!("ifg: warning: could not list members: {error}");
            }
            Ok(())
        }
        OrgCommand::Create {
            name,
            display,
            description,
            private,
        } => {
            let options = ifg_client::CreateOrgOptions {
                name: name.clone(),
                display_name: display.clone(),
                description: description.clone(),
                visibility: private.then(|| "private".to_string()),
            };
            if cli.json || cli.template.is_some() {
                let value = orgs.create_raw(&options).await?;
                match &cli.template {
                    Some(template) => {
                        output::print_template(std::slice::from_ref(&value), template)
                    }
                    None => output::print_json(&value),
                }
                return Ok(());
            }
            let org = orgs.create(&options).await?;
            println!("Created organization {} (id {})", org.name, org.id);
            Ok(())
        }
        OrgCommand::Members { name } => {
            if cli.json || cli.template.is_some() {
                let value = orgs.members_raw(name).await?;
                match &cli.template {
                    Some(template) => output::print_template(
                        value.as_array().cloned().unwrap_or_default().as_slice(),
                        template,
                    ),
                    None => output::print_json(&value),
                }
                return Ok(());
            }
            let members = orgs.members(name).await?;
            if members.is_empty() {
                println!("No members in {name}.");
                return Ok(());
            }
            let rows: Vec<Vec<String>> = members
                .iter()
                .map(|member| {
                    vec![
                        member.label(),
                        member.display_name.clone().unwrap_or_default(),
                        member.role.clone(),
                        output::human_time(member.created_at.as_deref()),
                    ]
                })
                .collect();
            output::print_table(&["USER", "DISPLAY", "ROLE", "JOINED"], &rows);
            Ok(())
        }
        OrgCommand::AddMember {
            name,
            username,
            role,
        } => {
            if let Some(role) = role {
                if !matches!(role.as_str(), "owner" | "admin" | "member") {
                    return Err(AppError::Usage(format!(
                        "unknown role '{role}'; use owner, admin or member"
                    )));
                }
            }
            let user = ctx.client.users().get(username).await?;
            orgs.add_member(name, user.id, role.as_deref()).await?;
            println!(
                "Added {} to {}{}",
                user.username,
                name,
                role.as_deref()
                    .map(|r| format!(" as {r}"))
                    .unwrap_or_default()
            );
            Ok(())
        }
        OrgCommand::RemoveMember { name, username } => {
            let user = ctx.client.users().get(username).await?;
            orgs.remove_member(name, user.id).await?;
            println!("Removed {} from {}.", user.username, name);
            Ok(())
        }
    }
}
