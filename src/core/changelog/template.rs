use std::collections::{HashMap, HashSet};
use std::error::Error as ErrorImpl;

use regex::Regex;
use serde::Serialize;
use tera::{ast, Context as TeraContext, Result as TeraResult, Tera, Value};

use super::config::TextProcessor;
use super::error::{Error, Result};

#[derive(Debug)]
pub struct Template {
    name: String,
    tera: Tera,
    pub variables: Vec<String>,
}

impl Template {
    pub fn new(name: &str, mut content: String, trim: bool) -> Result<Self> {
        if trim {
            content = content
                .lines()
                .map(|v| v.trim())
                .collect::<Vec<&str>>()
                .join("\n");
        }
        let mut tera = Tera::default();
        if let Err(e) = tera.add_raw_template(name, &content) {
            let content_snippet = content.lines().take(3).collect::<Vec<_>>().join("\n");
            return if let Some(error_source) = e.source() {
                Err(Error::TemplateParseError(format!(
                    "Template '{}' failed to parse: {}\nContent snippet:\n{}",
                    name, error_source, content_snippet
                )))
            } else {
                Err(Error::TemplateParseError(format!(
                    "Template '{}' failed to parse: {}\nContent snippet:\n{}",
                    name, e, content_snippet
                )))
            };
        }

        tera.register_filter("upper_first", Self::upper_first_filter);
        tera.register_filter("split_regex", Self::split_regex);
        tera.register_filter("replace_regex", Self::replace_regex);
        tera.register_filter("find_regex", Self::find_regex);

        Ok(Self {
            name: name.to_string(),
            variables: Self::get_template_variables(name, &tera)?,
            tera,
        })
    }

    fn upper_first_filter(value: &Value, _: &HashMap<String, Value>) -> TeraResult<Value> {
        let mut s = tera::try_get_value!("upper_first_filter", "value", String, value);
        let mut c = s.chars();
        s = match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        };
        Ok(tera::to_value(&s)?)
    }

    fn replace_regex(value: &Value, args: &HashMap<String, Value>) -> TeraResult<Value> {
        let s = tera::try_get_value!("replace_regex", "value", String, value);
        let from = match args.get("from") {
            Some(val) => tera::try_get_value!("replace_regex", "from", String, val),
            None => {
                return Err(tera::Error::msg(
                    "Filter `replace_regex` expected an arg called `from`",
                ));
            }
        };

        let to = match args.get("to") {
            Some(val) => tera::try_get_value!("replace_regex", "to", String, val),
            None => {
                return Err(tera::Error::msg(
                    "Filter `replace_regex` expected an arg called `to`",
                ));
            }
        };

        let re = Regex::new(&from).map_err(|e| {
            tera::Error::msg(format!(
                "Filter `replace_regex` received an invalid regex pattern: {}",
                e
            ))
        })?;
        Ok(tera::to_value(re.replace_all(&s, &to))?)
    }

    fn find_regex(value: &Value, args: &HashMap<String, Value>) -> TeraResult<Value> {
        let s = tera::try_get_value!("find_regex", "value", String, value);

        let pat = match args.get("pat") {
            Some(p) => {
                let p = tera::try_get_value!("find_regex", "pat", String, p);
                p.replace("\\n", "\n").replace("\\t", "\t")
            }
            None => {
                return Err(tera::Error::msg(
                    "Filter `find_regex` expected an arg called `pat`",
                ));
            }
        };
        let re = Regex::new(&pat).map_err(|e| {
            tera::Error::msg(format!(
                "Filter `find_regex` received an invalid regex pattern: {}",
                e
            ))
        })?;
        let result: Vec<&str> = re.find_iter(&s).map(|mat| mat.as_str()).collect();
        Ok(tera::to_value(result)?)
    }

    fn split_regex(value: &Value, args: &HashMap<String, Value>) -> TeraResult<Value> {
        let s = tera::try_get_value!("split_regex", "value", String, value);
        let pat = match args.get("pat") {
            Some(p) => {
                let p = tera::try_get_value!("split_regex", "pat", String, p);
                p.replace("\\n", "\n").replace("\\t", "\t")
            }
            None => {
                return Err(tera::Error::msg(
                    "Filter `split_regex` expected an arg called `pat`",
                ));
            }
        };
        let re = Regex::new(&pat).map_err(|e| {
            tera::Error::msg(format!(
                "Filter `split_regex` received an invalid regex pattern: {}",
                e
            ))
        })?;
        let result: Vec<&str> = re.split(&s).collect();
        Ok(tera::to_value(result)?)
    }

    fn find_identifiers(node: &ast::Node, names: &mut HashSet<String>) {
        match node {
            ast::Node::Block(_, block, _) => {
                for node in &block.body {
                    Self::find_identifiers(node, names);
                }
            }
            ast::Node::VariableBlock(_, expr) => {
                if let ast::ExprVal::Ident(v) = &expr.val {
                    names.insert(v.clone());
                }
            }
            ast::Node::MacroDefinition(_, def, _) => {
                for node in &def.body {
                    Self::find_identifiers(node, names);
                }
            }
            ast::Node::FilterSection(_, section, _) => {
                for node in &section.body {
                    Self::find_identifiers(node, names);
                }
            }
            ast::Node::Forloop(_, forloop, _) => {
                if let ast::ExprVal::Ident(v) = &forloop.container.val {
                    names.insert(v.clone());
                }
                for node in &forloop.body {
                    Self::find_identifiers(node, names);
                }
                for node in &forloop.empty_body.clone().unwrap_or_default() {
                    Self::find_identifiers(node, names);
                }
                for (_, expr) in forloop.container.filters.iter().flat_map(|v| v.args.iter()) {
                    if let ast::ExprVal::String(ref v) = expr.val {
                        names.insert(v.clone());
                    }
                }
            }
            ast::Node::If(cond, _) => {
                for (_, expr, nodes) in &cond.conditions {
                    if let ast::ExprVal::Ident(v) = &expr.val {
                        names.insert(v.clone());
                    }
                    for node in nodes {
                        Self::find_identifiers(node, names);
                    }
                }
                if let Some((_, nodes)) = &cond.otherwise {
                    for node in nodes {
                        Self::find_identifiers(node, names);
                    }
                }
            }
            _ => {}
        }
    }

    fn get_template_variables(name: &str, tera: &Tera) -> Result<Vec<String>> {
        let mut variables = HashSet::new();
        let ast = &tera.get_template(name)?.ast;
        for node in ast {
            Self::find_identifiers(node, &mut variables);
        }
        log::trace!("Template variables for {name}: {variables:?}");
        Ok(variables.into_iter().collect())
    }

    pub fn contains_variable(&self, variables: &[&str]) -> bool {
        variables
            .iter()
            .any(|var| self.variables.iter().any(|v| v.starts_with(var)))
    }

    pub fn render<C: Serialize, T: Serialize, S: Into<String> + Clone>(
        &self,
        context: &C,
        additional_context: Option<&HashMap<S, T>>,
        postprocessors: &[TextProcessor],
    ) -> Result<String> {
        let mut context = TeraContext::from_serialize(context)?;
        if let Some(additional_context) = additional_context {
            for (key, value) in additional_context {
                context.insert(key.clone(), &value);
            }
        }
        match self.tera.render(&self.name, &context) {
            Ok(mut v) => {
                for postprocessor in postprocessors {
                    postprocessor.replace(&mut v, vec![])?;
                }
                Ok(v)
            }
            Err(e) => {
                if let Some(source1) = e.source() {
                    if let Some(source2) = source1.source() {
                        Err(Error::TemplateRenderError(format!(
                            "Template '{}' render failed: {}: {}",
                            self.name, source1, source2
                        )))
                    } else {
                        Err(Error::TemplateRenderError(format!(
                            "Template '{}' render failed: {}",
                            self.name, source1
                        )))
                    }
                } else {
                    Err(Error::TemplateRenderError(format!(
                        "Template '{}' render failed: {}",
                        self.name, e
                    )))
                }
            }
        }
    }
}
