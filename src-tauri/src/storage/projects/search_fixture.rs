//! Fixed metadata only. No filesystem grants; debug/test callers only.
use super::*;

pub(crate) fn seed(connection: &mut Connection) -> Result<(), ErrorCode> {
    let tx = connection.transaction().map_err(sql)?;
    for index in 0..1000 {
        let id = format!("00000000-0000-4000-8000-{index:012x}");
        let name = match index {
            0 => "项目助手".into(),
            1 => "项目桌面助手".into(),
            2 => "工具项目助手扩展".into(),
            _ => format!("Synthetic-{index:04}"),
        };
        let display = (index == 3).then_some("中文工作台");
        // Deliberately ungranted synthetic rows, never valid for system open.
        tx.execute("INSERT INTO projects(id,canonical_path,path_key,name,display_name,created_at,updated_at) VALUES(?1,?2,?2,?3,?4,1,1)",
            params![id, format!("C:\\synthetic-private-path\\{name}"), name, display]).map_err(sql)?;
        if index % 3 == 0 {
            tx.execute("INSERT INTO project_usage(project_id,last_opened_at,open_count,last_source) VALUES(?1,?2,1,'project_list')",
                params![id, index / 10 + 1]).map_err(sql)?;
        }
    }
    tx.commit().map_err(sql)
}
