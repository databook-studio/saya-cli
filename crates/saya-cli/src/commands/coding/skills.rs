use super::{CodingApplication, CodingError};
use saya_harness::{
    coding::skills::{
        SkillOrigin, SkillRoot, discover_skills, load_skill_body, load_skill_resources,
    },
    workspace::Workspace,
};
use serde_json::{Value, json};
impl CodingApplication {
    pub(crate) fn skills(
        &self,
        name: Option<&str>,
        resources: &[String],
    ) -> Result<Value, CodingError> {
        let path = self.repository.root().join(".agents/skills");
        if !path.exists() {
            return Ok(json!({"skills":[]}));
        }
        if std::fs::symlink_metadata(&path)
            .map_err(CodingError::owner)?
            .file_type()
            .is_symlink()
            || !path
                .canonicalize()
                .map_err(CodingError::owner)?
                .starts_with(self.repository.root())
        {
            return Err(CodingError::NotOwned);
        }
        let roots = vec![
            SkillRoot::new(
                Workspace::open(&path).map_err(CodingError::owner)?,
                SkillOrigin::Project,
                "untrusted original-project skill",
            )
            .map_err(CodingError::owner)?,
        ];
        let index = discover_skills(&roots).map_err(CodingError::owner)?;
        let Some(name) = name else {
            return Ok(
                json!({"skills":index.skills.iter().map(|entry|json!({"name":entry.name,"description":entry.description,"metadata_sha256":entry.metadata_sha256})).collect::<Vec<_>>(),"authority":"none"}),
            );
        };
        let entry = index
            .skills
            .iter()
            .find(|entry| entry.name == name)
            .ok_or(CodingError::InvalidRequest)?;
        let body = load_skill_body(&roots, entry).map_err(CodingError::owner)?;
        let resources =
            load_skill_resources(&roots, entry, resources).map_err(CodingError::owner)?;
        Ok(
            json!({"name":body.name(),"body":body.text(),"sha256":body.source_sha256(),"authority":"none","resources":resources.iter().map(|resource|json!({"reference":resource.reference(),"text":resource.text(),"sha256":resource.source_sha256()})).collect::<Vec<_>>()}),
        )
    }
}
