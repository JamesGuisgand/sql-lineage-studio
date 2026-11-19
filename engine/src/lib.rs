//! SQL lineage based on sqlparser-rs AST.
//!
//! Static analysis only: no SQL is executed and no database connection is made.
//! Columns without an unambiguous origin are marked unresolved rather than guessed.

use serde::Serialize;
use sqlparser::ast::{
    visit_expressions, visit_relations, Expr, FromTable, Ident, ObjectName, Query, Select, SelectItem,
    SetExpr, Statement as AstStatement, TableFactor, TableWithJoins,
};
use sqlparser::dialect::{GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::ControlFlow;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Statement {
    pub kind: String,
    pub target: Option<String>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ColumnEdge {
    pub source: String,
    pub target: String,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImpactPath {
    pub target: String,
    pub distance: usize,
    pub path: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Impact {
    pub start: String,
    pub affected: Vec<String>,
    pub paths: Vec<ImpactPath>,
    pub cycles: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub statements: Vec<Statement>,
    pub columns: Vec<ColumnEdge>,
    pub warnings: Vec<String>,
    pub impact: Option<Impact>,
    pub dot: String,
}

#[derive(Debug, Clone, Default)]
struct Projection {
    name: String,
    sources: BTreeSet<String>,
    uncertain: bool,
}

#[derive(Debug, Clone, Default)]
struct Relation {
    instance: usize,
    name: String,
    // None: physical relation with unknown schema, Some: derived/CTE with known projection
    columns: Option<Vec<Projection>>,
}

#[derive(Debug, Clone, Default)]
struct QueryResult {
    projections: Vec<Projection>,
    tables: BTreeSet<String>,
}

struct Context<'a> {
    catalog: &'a BTreeMap<String, Vec<Projection>>,
    warnings: &'a mut BTreeSet<String>,
}

type Scope = BTreeMap<String, Relation>;

fn normalize(name: &str) -> String { name.trim().to_string() }
fn obj(name: &ObjectName) -> String { name.0.iter().map(ident).collect::<Vec<_>>().join(".") }
fn ident(name: &Ident) -> String {
    if name.quote_style.is_some() { name.value.clone() }
    else { name.value.to_lowercase() }
}

fn parse(sql: &str, dialect: &str) -> Result<Vec<AstStatement>, String> {
    let result = match dialect {
        "generic" => Parser::parse_sql(&GenericDialect {}, sql),
        "postgres" => Parser::parse_sql(&PostgreSqlDialect {}, sql),
        "mysql" => Parser::parse_sql(&MySqlDialect {}, sql),
        "sqlite" => Parser::parse_sql(&SQLiteDialect {}, sql),
        _ => return Err(format!("Dialecte inconnu : {dialect}. Choisir generic, postgres, mysql, sqlite")),
    };
    result.map_err(|e| format!("Erreur de syntaxe SQL : {e}"))
}

// A visitor is used for physical relations, including subqueries, joins, and set operations.
// CTE names must be removed: they are intermediate SQL aliases, not database tables.
fn referenced_tables(query: &Query, cte_names: &BTreeSet<String>) -> BTreeSet<String> {
    let mut tables = BTreeSet::new();
    let _ = visit_relations(query, |name| {
        let n = obj(name);
        if !cte_names.contains(&n) { tables.insert(n); }
        ControlFlow::<()>::Continue(())
    });
    tables
}

fn collect_cte_names(query: &Query, known: &mut BTreeSet<String>) {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            known.insert(ident(&cte.alias.name));
            collect_cte_names(&cte.query, known);
        }
    }
    // visitor visits query recursively in the relation pass, but CTE aliases in derived
    // subqueries remain outside this local set; analyze_query also collects their sources.
}

fn add_relation(
    factor: &TableFactor, scope: &mut Scope, ctes: &Scope, catalog: &BTreeMap<String, Vec<Projection>>,
    ctx: &mut Context<'_>, sources: &mut BTreeSet<String>,
) {
    match factor {
        TableFactor::Table { name, alias, .. } => {
            let table = obj(name);
            let mut rel = ctes.get(&table).cloned().unwrap_or_else(|| Relation {
                instance: 0, name: table.clone(), columns: catalog.get(&table).cloned(),
            });
            rel.instance = scope.len() + 1;
            if !ctes.contains_key(&table) { sources.insert(table.clone()); }
            let alias_name = alias.as_ref().map(|a| ident(&a.name))
                .unwrap_or_else(|| table.split('.').last().unwrap_or(&table).to_string());
            scope.insert(alias_name, rel.clone());
            scope.insert(table, rel);
        }
        TableFactor::Derived { subquery, alias, .. } => {
            let query = analyze_query(subquery, ctes, ctx);
            sources.extend(query.tables);
            let derived = Relation { instance: scope.len() + 1, name: "(derived)".into(), columns: Some(query.projections) };
            if let Some(a) = alias { scope.insert(ident(&a.name), derived); }
            else { ctx.warnings.insert("Sous-requête FROM sans alias : lineage des colonnes non résolu".into()); }
        }
        TableFactor::NestedJoin { table_with_joins, .. } => {
            add_joined(table_with_joins, scope, ctes, catalog, ctx, sources);
        }
        _ => { ctx.warnings.insert("Type de relation SQL non pris en charge pour la résolution des colonnes".into()); }
    }
}

fn add_joined(
    joined: &TableWithJoins, scope: &mut Scope, ctes: &Scope,
    catalog: &BTreeMap<String, Vec<Projection>>,
    ctx: &mut Context<'_>, sources: &mut BTreeSet<String>,
) {
    add_relation(&joined.relation, scope, ctes, catalog, ctx, sources);
    for join in &joined.joins {
        add_relation(&join.relation, scope, ctes, catalog, ctx, sources);
    }
}

fn map_column(rel: &Relation, column: &str, warnings: &mut BTreeSet<String>) -> BTreeSet<String> {
    if let Some(projections) = &rel.columns {
        if let Some(mapped) = projections.iter().find(|p| p.name == column) {
            let mut output = mapped.sources.clone();
            // Preserve direct view -> downstream view dependency as well as
            // transitive physical origins. CTE/derived aliases are not persisted.
            if !rel.name.starts_with('(') { output.insert(format!("{}.{}", rel.name, column)); }
            return output;
        }
        if projections.iter().any(|p| p.name == "*") {
            warnings.insert(format!("Colonne {column} issue d'un SELECT * : schéma nécessaire"));
            return BTreeSet::new();
        }
        warnings.insert(format!("Colonne {column} non trouvée dans les projections de {}", rel.name));
        BTreeSet::new()
    } else {
        BTreeSet::from([format!("{}.{}", rel.name, column)])
    }
}

fn column_refs(expr: &Expr, scope: &Scope, warnings: &mut BTreeSet<String>) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    // The AST visitor traverses functions, arithmetic expressions, CASE and nested expressions.
    let _ = visit_expressions(expr, |node| {
        match node {
            Expr::Identifier(col) => {
                let col = ident(col);
                // Scope contains both base names and aliases. Deduplicate actual relations.
                let unique: BTreeMap<usize, &Relation> = scope.values()
                    .map(|rel| (rel.instance, rel)).collect();
                if unique.len() == 1 {
                    if let Some(rel) = unique.values().next() {
                        result.extend(map_column(rel, &col, warnings));
                    }
                } else if unique.is_empty() {
                    warnings.insert(format!("Colonne {col} sans relation FROM : origine non résolue"));
                } else {
                    warnings.insert(format!("Colonne non qualifiée ambiguë : {col} ({} relations)", unique.len()));
                }
            }
            Expr::CompoundIdentifier(ids) if ids.len() >= 2 => {
                let col = ident(ids.last().expect("len checked"));
                let qualifier = ids[..ids.len()-1].iter().map(ident).collect::<Vec<_>>().join(".");
                if let Some(rel) = scope.get(&qualifier) {
                    result.extend(map_column(rel, &col, warnings));
                } else {
                    warnings.insert(format!("Alias ou table introuvable : {qualifier}.{col}"));
                }
            }
            _ if node.to_string() == "*" => {
                warnings.insert("COUNT(*) ou expression joker : dépendance au niveau des lignes, pas d'une colonne précise".into());
            }
            _ => {}
        }
        ControlFlow::<()>::Continue(())
    });
    result
}

fn analyze_select(
    select: &Select, ctes: &Scope, ctx: &mut Context<'_>,
) -> QueryResult {
    let mut scope = Scope::new();
    let mut tables = BTreeSet::new();
    for joined in &select.from {
        add_joined(joined, &mut scope, ctes, ctx.catalog, ctx, &mut tables);
    }
    let mut projections = Vec::new();
    for (index, projection) in select.projection.iter().enumerate() {
        match projection {
            SelectItem::ExprWithAlias { expr, alias } => projections.push(Projection {
                name: ident(alias), sources: column_refs(expr, &scope, ctx.warnings), uncertain: false,
            }),
            SelectItem::UnnamedExpr(expr) => {
                let name = match expr {
                    Expr::Identifier(id) => ident(id),
                    Expr::CompoundIdentifier(ids) => ident(ids.last().expect("not empty")),
                    _ => format!("expression_{}", index+1),
                };
                projections.push(Projection { name, sources: column_refs(expr, &scope, ctx.warnings), uncertain: false });
            }
            SelectItem::Wildcard(_) => {
                ctx.warnings.insert("SELECT * : noms et dépendances de colonnes non exhaustifs sans schéma".into());
                projections.push(Projection { name: "*".into(), sources: BTreeSet::new(), uncertain: true });
            }
            SelectItem::QualifiedWildcard(name, _) => {
                ctx.warnings.insert(format!("{}. * : projection non résolue sans schéma", obj(name)));
                projections.push(Projection { name: "*".into(), sources: BTreeSet::new(), uncertain: true });
            }
        }
    }
    QueryResult { projections, tables }
}

fn analyze_query(query: &Query, inherited: &Scope, ctx: &mut Context<'_>) -> QueryResult {
    let mut ctes = inherited.clone();
    let mut tables = BTreeSet::new();
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            let mut result = analyze_query(&cte.query, &ctes, ctx);
            tables.append(&mut result.tables);
            for (pos, alias_col) in cte.alias.columns.iter().enumerate() {
                if let Some(projected) = result.projections.get_mut(pos) {
                    projected.name = ident(alias_col);
                }
            }
            ctes.insert(ident(&cte.alias.name), Relation {
                instance: 0, name: format!("(cte:{})", ident(&cte.alias.name)), columns: Some(result.projections),
            });
        }
    }
    let result = match query.body.as_ref() {
        SetExpr::Select(select) => analyze_select(select, &ctes, ctx),
        SetExpr::Query(q) => analyze_query(q, &ctes, ctx),
        SetExpr::SetOperation { left, right, .. } => {
            let left = analyze_setexpr(left, &ctes, ctx);
            let right = analyze_setexpr(right, &ctes, ctx);
            let mut projections = left.projections;
            if projections.len() == right.projections.len() {
                for (a,b) in projections.iter_mut().zip(right.projections) {
                    a.sources.extend(b.sources);
                    a.uncertain |= b.uncertain;
                }
            } else {
                ctx.warnings.insert("UNION/INTERSECT/EXCEPT : projections de tailles différentes".into());
            }
            QueryResult { projections, tables: left.tables.union(&right.tables).cloned().collect() }
        }
        _ => {
            ctx.warnings.insert("Expression de requête SQL non prise en charge pour le lineage des colonnes".into());
            QueryResult::default()
        }
    };
    tables.extend(result.tables);
    // Gather every physical table in the actual AST, including correlated/EXISTS/subqueries
    // that don't occur in FROM or JOIN at the immediate SELECT level.
    let mut cte_names: BTreeSet<String> = ctes.keys().cloned().collect();
    collect_cte_names(query, &mut cte_names);
    tables.extend(referenced_tables(query, &cte_names));
    QueryResult { projections: result.projections, tables }
}

fn analyze_setexpr(body: &SetExpr, ctes: &Scope, ctx: &mut Context<'_>) -> QueryResult {
    match body {
        SetExpr::Select(s) => analyze_select(s, ctes, ctx),
        SetExpr::Query(q) => analyze_query(q, ctes, ctx),
        SetExpr::SetOperation {left, right, ..} => {
            let mut lhs = analyze_setexpr(left, ctes, ctx);
            let rhs = analyze_setexpr(right, ctes, ctx);
            for (a,b) in lhs.projections.iter_mut().zip(rhs.projections) {
                a.sources.extend(b.sources);
                a.uncertain |= b.uncertain;
            }
            lhs.tables.extend(rhs.tables);
            lhs
        }
        _ => QueryResult::default(),
    }
}

fn analyze_statement(
    ast: &AstStatement, catalog: &BTreeMap<String, Vec<Projection>>,
    warnings: &mut BTreeSet<String>,
) -> Result<(Statement, Vec<Projection>), String> {
    let mut ctx = Context { catalog, warnings };
    let (kind, target, result, output_names) = match ast {
        AstStatement::Query(q) => ("select", None, analyze_query(q, &Scope::new(), &mut ctx), vec![]),
        AstStatement::CreateView {name, query, columns, ..} => (
            "create_view", Some(obj(name)), analyze_query(query, &Scope::new(), &mut ctx),
            columns.iter().map(|column| ident(&column.name)).collect(),
        ),
        AstStatement::CreateTable {name, query: Some(query), ..} => (
            "create_table_as", Some(obj(name)), analyze_query(query, &Scope::new(), &mut ctx), vec![],
        ),
        AstStatement::Insert(insert) => {
            let Some(query) = &insert.source else {
                return Err("INSERT VALUES sans SELECT : aucune dépendance de source à analyser".into());
            };
            ("insert", Some(obj(&insert.table_name)), analyze_query(query, &Scope::new(), &mut ctx),
             insert.columns.iter().map(ident).collect())
        }
        AstStatement::Update { table, .. } => {
            let target = match &table.relation {
                TableFactor::Table { name, .. } => obj(name),
                _ => return Err("UPDATE : table cible non reconnue".into()),
            };
            let mut sources = BTreeSet::new();
            let _ = visit_relations(ast, |name| {
                sources.insert(obj(name));
                ControlFlow::<()>::Continue(())
            });
            sources.remove(&target);
            ctx.warnings.insert("UPDATE : dépendances de tables uniquement ; affectations de colonnes non résolues".into());
            return Ok((Statement { kind: "update".into(), target: Some(target),
                sources: sources.into_iter().collect() }, vec![]));
        }
        AstStatement::Delete(delete) => {
            let tables = match &delete.from {
                FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t) => t,
            };
            let target = match tables.first().map(|t| &t.relation) {
                Some(TableFactor::Table { name, .. }) => obj(name),
                _ => return Err("DELETE : table cible non reconnue".into()),
            };
            let mut sources = BTreeSet::new();
            let _ = visit_relations(ast, |name| {
                sources.insert(obj(name));
                ControlFlow::<()>::Continue(())
            });
            sources.remove(&target);
            ctx.warnings.insert("DELETE : dépendances de tables uniquement ; filtres non interprétés au niveau des colonnes".into());
            return Ok((Statement { kind: "delete".into(), target: Some(target),
                sources: sources.into_iter().collect() }, vec![]));
        }
        _ => return Err(format!("Instruction non prise en charge : {}", ast.to_string().split_whitespace().next().unwrap_or("SQL"))),
    };
    let mut projections = result.projections;
    for (index, output) in output_names.iter().enumerate() {
        if let Some(projection) = projections.get_mut(index) { projection.name = output.to_string(); }
    }
    let mut sources = result.tables;
    if let Some(ref t) = target { sources.remove(t); }
    Ok((Statement { kind: kind.into(), target, sources: sources.into_iter().collect() }, projections))
}

pub fn analyze(sql: &str, dialect: &str, impact_start: Option<&str>) -> Result<Report, String> {
    let asts = parse(sql, dialect)?;
    if asts.is_empty() { return Err("Aucune instruction SQL détectée".into()); }
    if asts.len() > 500 { return Err("Trop d'instructions : limite 500".into()); }
    let mut statements = Vec::new();
    let mut catalog: BTreeMap<String, Vec<Projection>> = BTreeMap::new();
    let mut warnings = BTreeSet::new();
    let mut columns = BTreeSet::new();
    for (index, ast) in asts.iter().enumerate() {
        let (statement, projections) = analyze_statement(ast, &catalog, &mut warnings)
            .map_err(|err| format!("Instruction {} : {err}", index + 1))?;
        if let Some(ref target) = statement.target {
            for projection in &projections {
                if projection.uncertain {
                    warnings.insert(format!("{}.{} : origine non résolue (joker *)", target, projection.name));
                }
                for origin in &projection.sources {
                    columns.insert(ColumnEdge {
                        source: origin.clone(), target: format!("{}.{}", target, projection.name),
                        confidence: if projection.uncertain { "partial" } else { "resolved" }.into(),
                    });
                }
            }
            catalog.insert(target.to_string(), projections);
        }
        statements.push(statement);
    }
    let dot = dot_output(&statements);
    let impact = impact_start.map(|table| calculate_impact(&statements, table));
    Ok(Report { statements, columns: columns.into_iter().collect(),
                warnings: warnings.into_iter().collect(), impact, dot })
}

pub fn dot_output(statements: &[Statement]) -> String {
    let mut edges = BTreeSet::new();
    let mut nodes = BTreeSet::new();
    for stmt in statements {
        if let Some(target) = &stmt.target {
            nodes.insert(target.clone());
            for source in &stmt.sources {
                edges.insert((source.clone(), target.clone()));
                nodes.insert(source.clone());
            }
        } else {
            nodes.extend(stmt.sources.iter().cloned());
        }
    }
    let mut result = String::from("digraph lineage {\n  rankdir=LR;\n  node [shape=box];\n");
    for node in nodes { result.push_str(&format!("  {};\n", dot_quote(&node))); }
    for (from, to) in edges { result.push_str(&format!("  {} -> {};\n", dot_quote(&from), dot_quote(&to))); }
    result.push_str("}\n");
    result
}
fn dot_quote(s: &str) -> String { serde_json::to_string(s).expect("escape string") }

pub fn calculate_impact(statements: &[Statement], table: &str) -> Impact {
    let start = normalize(table);
    let mut graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for stmt in statements {
        if let Some(target) = &stmt.target {
            for src in &stmt.sources { graph.entry(src.clone()).or_default().insert(target.clone()); }
        }
    }
    let mut seen = BTreeMap::new();
    seen.insert(start.clone(), vec![start.clone()]);
    let mut queue = VecDeque::from([start.clone()]);
    while let Some(current) = queue.pop_front() {
        let path = seen.get(&current).expect("queued").clone();
        if let Some(neighbors) = graph.get(&current) {
            for next in neighbors {
                if !seen.contains_key(next) {
                    let mut next_path = path.clone();
                    next_path.push(next.to_string());
                    seen.insert(next.clone(), next_path);
                    queue.push_back(next.clone());
                }
            }
        }
    }
    let paths = seen.iter().filter(|(name,_)| *name != &start).map(|(name, path)| ImpactPath {
        target: name.clone(), distance: path.len()-1, path: path.clone(),
    }).collect::<Vec<_>>();
    let affected = paths.iter().map(|p| p.target.clone()).collect::<Vec<_>>();
    let cycles = find_cycles(&graph);
    Impact { start, affected, paths, cycles }
}

fn find_cycles(graph: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    // DFS with active-stack cycle detection, canonicalized for deduplication.
    fn dfs(node: &str, graph: &BTreeMap<String, BTreeSet<String>>,
           visited: &mut BTreeSet<String>, stack: &mut Vec<String>,
           active: &mut BTreeSet<String>, cycles: &mut BTreeSet<Vec<String>>) {
        visited.insert(node.to_string());
        active.insert(node.to_string());
        stack.push(node.to_string());
        if let Some(nexts) = graph.get(node) {
            for next in nexts {
                if active.contains(next) {
                    if let Some(at) = stack.iter().position(|v| v == next) {
                        let cycle = stack[at..].to_vec();
                        if let Some((min_index, _)) = cycle.iter().enumerate().min_by_key(|(_,x)| *x) {
                            let mut normalized = cycle[min_index..].to_vec();
                            normalized.extend_from_slice(&cycle[..min_index]);
                            normalized.push(normalized[0].clone());
                            cycles.insert(normalized);
                        }
                    }
                } else if !visited.contains(next) {
                    dfs(next, graph, visited, stack, active, cycles);
                }
            }
        }
        stack.pop();
        active.remove(node);
    }
    let mut visited = BTreeSet::new();
    let mut active = BTreeSet::new();
    let mut stack = Vec::new();
    let mut cycles = BTreeSet::new();
    let nodes = graph.keys().cloned().collect::<Vec<_>>();
    for node in nodes {
        if !visited.contains(&node) {
            dfs(&node, graph, &mut visited, &mut stack, &mut active, &mut cycles);
        }
    }
    cycles.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn analyze_test(sql: &str) -> Report { analyze(sql, "generic", None).unwrap() }
    #[test] fn joins_schema_names() {
        let r = analyze_test("SELECT o.id FROM sales.orders o JOIN crm.customers c ON c.id=o.customer_id");
        assert_eq!(r.statements[0].sources, vec!["crm.customers", "sales.orders"]);
    }
    #[test] fn quoted_identifiers_are_case_sensitive() {
        let r = analyze_test("SELECT o.\"Price\" FROM \"Sales\".\"Orders\" o");
        assert_eq!(r.statements[0].sources, vec!["Sales.Orders"]);
    }
    #[test] fn projection_qualified() {
        let r = analyze_test("CREATE VIEW reporting.ventes AS SELECT o.price * o.qty AS total FROM sales.orders o");
        assert!(r.columns.iter().any(|e| e.source == "sales.orders.price" && e.target == "reporting.ventes.total"));
        assert!(r.columns.iter().any(|e| e.source == "sales.orders.qty" && e.target == "reporting.ventes.total"));
    }
    #[test] fn cte_resolves_columns() {
        let r = analyze_test("CREATE VIEW reporting.x AS WITH recent AS (SELECT o.amount FROM sales.orders o) SELECT recent.amount FROM recent");
        assert!(r.columns.iter().any(|e| e.source == "sales.orders.amount" && e.target == "reporting.x.amount"));
        assert_eq!(r.statements[0].sources, vec!["sales.orders"]);
    }
    #[test] fn derived_select() {
        let r = analyze_test("CREATE VIEW reports.x AS SELECT t.price FROM (SELECT p.price FROM raw.products p) t");
        assert!(r.columns.iter().any(|e| e.source == "raw.products.price" && e.target == "reports.x.price"));
    }
    #[test] fn multi_statement_catalog() {
        let r = analyze_test("CREATE VIEW v AS SELECT a.id FROM base a; CREATE VIEW w AS SELECT v.id FROM v;");
        assert!(r.columns.iter().any(|e| e.source == "base.id" && e.target == "w.id"));
        assert!(r.statements[1].sources.contains(&"v".to_string()));
    }
    #[test] fn union_combines() {
        let r = analyze_test("CREATE VIEW v AS SELECT x.id FROM a x UNION ALL SELECT y.id FROM b y");
        assert!(r.columns.iter().any(|e| e.source == "a.id" && e.target == "v.id"));
        assert!(r.columns.iter().any(|e| e.source == "b.id" && e.target == "v.id"));
    }
    #[test] fn wildcard_explicit_warning() {
        let r = analyze_test("CREATE VIEW v AS SELECT * FROM raw.orders");
        assert!(r.columns.is_empty());
        assert!(r.warnings.iter().any(|warning| warning.contains("SELECT *")));
    }
    #[test] fn ambiguous_column_explicit_warning() {
        let r = analyze_test("CREATE VIEW v AS SELECT id FROM a JOIN b ON a.id=b.id");
        assert!(r.columns.is_empty());
        assert!(r.warnings.iter().any(|w| w.contains("ambiguë")));
    }
    #[test] fn self_join_unqualified_is_ambiguous() {
        let r = analyze_test("CREATE VIEW v AS SELECT id FROM users a JOIN users b ON a.id=b.id");
        assert!(r.columns.is_empty());
        assert!(r.warnings.iter().any(|w| w.contains("ambiguë")));
    }
    #[test] fn sql_in_literal_protected_by_ast() {
        let r = analyze_test("SELECT 'FROM fake' AS label FROM real_table");
        assert_eq!(r.statements[0].sources, vec!["real_table"]);
    }
    #[test] fn impact_transitive_paths() {
        let r = analyze_test("CREATE VIEW b AS SELECT id FROM a; CREATE VIEW c AS SELECT id FROM b;");
        let impact = calculate_impact(&r.statements, "a");
        assert_eq!(impact.affected, vec!["b", "c"]);
        assert_eq!(impact.paths.iter().find(|p| p.target == "c").unwrap().distance, 2);
    }
    #[test] fn impact_cycles() {
        let statements = vec![Statement{kind:"insert".into(),target:Some("a".into()), sources:vec!["b".into()]},
                              Statement{kind:"insert".into(),target:Some("b".into()), sources:vec!["a".into()]}];
        assert!(!calculate_impact(&statements, "a").cycles.is_empty());
    }
    #[test] fn update_from_sources() {
        let r = analyze_test("UPDATE target SET amount = src.amount FROM source src WHERE target.id = src.id");
        assert_eq!(r.statements[0].target.as_deref(), Some("target"));
        assert_eq!(r.statements[0].sources, vec!["source"]);
    }
    #[test] fn delete_using_sources() {
        let r = analyze_test("DELETE FROM target USING source WHERE target.id = source.id");
        assert_eq!(r.statements[0].target.as_deref(), Some("target"));
        assert_eq!(r.statements[0].sources, vec!["source"]);
    }
    #[test] fn rejected_statement_explicit() {
        assert!(analyze("DROP TABLE t", "generic", None).is_err());
    }
    #[test] fn invalid_sql_is_rejected() {
        assert!(analyze("SELECT 'broken", "generic", None).is_err());
    }
    #[test] fn json_serializable() {
        let r = analyze_test("CREATE VIEW v AS SELECT a.id FROM t a");
        let raw = serde_json::to_string(&r).unwrap();
        assert!(raw.contains("\"statements\""));
    }
}
