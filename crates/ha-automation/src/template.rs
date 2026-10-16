//! Les **modèles** : des valeurs dans les textes des actions, et des calculs.
//!
//! ```yaml
//! - action: notify.send_message
//!   target: {entity_id: notify.telephone}
//!   data:
//!     message: "Il fait {{ states('sensor.meteo_temperature') }} °C dehors."
//! ```
//!
//! On reprend l'écriture de Home Assistant (Jinja), mais seulement une
//! partie :
//!
//! - **lire** : l'état d'une entité, un attribut, ce qui a déclenché
//!   l'automatisation (`trigger.…`) ;
//! - **calculer** (étape 64) : `+ - * / // %`, `~` pour coller deux textes ;
//! - **comparer** : `== != < <= > >=`, `in` et `not in` (dans une liste ou un
//!   texte) ;
//! - **combiner** : `and`, `or`, `not`, `x if condition else y`, des
//!   parenthèses ;
//! - quelques **filtres** (`round`, `float`, `default`, `abs`…).
//!
//! Pas de boucles, pas de variables, pas d'accès aux fichiers : un modèle ne
//! peut rien faire d'autre que produire une valeur courte. Chaque modèle est
//! vérifié au chargement (fonctions et filtres connus, identifiants d'entités
//! valides, taille et imbrication limitées). À l'exécution, une valeur
//! introuvable ou un calcul impossible (diviser par zéro, additionner un texte
//! et un nombre) donne une valeur vide, ou celle de `default(…)` : jamais
//! d'erreur qui bloque l'automatisation.

use std::cmp::Ordering;

use ha_core::{Attributes, EntityId, State, StateMachine};
use serde_json::{Number, Value};

/// Longueur maximale d'un texte qui contient un modèle, et du résultat.
const MAX_LEN: usize = 2000;
/// Nombre maximal de `{{ … }}` dans un texte.
const MAX_EXPRESSIONS: usize = 20;
/// Nombre maximal de filtres à la suite.
const MAX_FILTERS: usize = 5;
/// Imbrication maximale (parenthèses, `not`, `-`, `if … else`).
const MAX_DEPTH: usize = 20;
/// Morceaux (valeurs, opérations, filtres) au plus dans une expression.
const MAX_NODES: usize = 200;
/// Éléments d'une liste au plus (écrite, ou lue dans un attribut).
const MAX_LIST: usize = 50;

/// Un texte avec des `{{ … }}`, vérifié.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(String),
    Expr(Node),
}

/// Une expression, en arbre : `a + b * 2` donne `Add(a, Mul(b, 2))`.
#[derive(Debug, Clone, PartialEq)]
enum Node {
    Base(Base),
    List(Vec<Node>),
    Filter(Box<Node>, Filter),
    Neg(Box<Node>),
    Not(Box<Node>),
    Binary(Op, Box<Node>, Box<Node>),
    If {
        condition: Box<Node>,
        then: Box<Node>,
        otherwise: Box<Node>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
enum Base {
    Literal(Val),
    /// `states('sensor.x')` : l'état, `unknown` si l'entité n'existe pas.
    States(EntityId),
    /// `state_attr('sensor.x', 'nom')` : un attribut, ou rien.
    StateAttr(EntityId, String),
    /// `is_state('sensor.x', 'on')` : vrai ou faux.
    IsState(EntityId, String),
    /// `trigger.…` : ce qui a déclenché l'automatisation.
    Trigger(TriggerField),
}

#[derive(Debug, Clone, PartialEq)]
enum TriggerField {
    Id,
    EntityId,
    State(Side, StateField),
    /// `trigger.event.event_type`.
    EventType,
    /// `trigger.event.data.nom`.
    EventData(String),
}

/// L'état d'avant (`from_state`) ou d'après (`to_state`) le changement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    From,
    To,
}

#[derive(Debug, Clone, PartialEq)]
enum StateField {
    State,
    Name,
    EntityId,
    Attribute(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Filter {
    Round(u8),
    Int(Option<Val>),
    Float(Option<Val>),
    Abs,
    Lower,
    Upper,
    Capitalize,
    Default(Val),
    Replace(String, String),
}

/// Une valeur calculée.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    /// Introuvable (entité ou attribut absent, nombre illisible, calcul
    /// impossible…).
    None,
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<Val>),
}

impl Val {
    fn from_json(value: &Value) -> Self {
        match value {
            Value::String(text) => Self::Text(text.clone()),
            Value::Number(number) => number.as_f64().map_or(Self::None, Self::Number),
            Value::Bool(flag) => Self::Bool(*flag),
            // Une liste (les personnes de la maison…) : pour `in`.
            Value::Array(items) => {
                Self::List(items.iter().take(MAX_LIST).map(Self::from_json).collect())
            }
            // Les objets : trop gros pour un message.
            Value::Null | Value::Object(_) => Self::None,
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::Text(text) => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
            Self::Bool(_) | Self::None | Self::List(_) => None,
        }
    }

    fn text(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Text(text) => text.clone(),
            Self::Number(number) => number_text(*number),
            Self::Bool(true) => "True".into(),
            Self::Bool(false) => "False".into(),
            Self::List(items) => {
                let inner: Vec<String> = items
                    .iter()
                    .map(|item| match item {
                        Self::Text(text) => format!("'{text}'"),
                        other => other.text(),
                    })
                    .collect();
                format!("[{}]", inner.join(", "))
            }
        }
    }

    /// Vrai ou faux, comme en Python : vide, zéro et `None` sont faux.
    fn truthy(&self) -> bool {
        match self {
            Self::None => false,
            Self::Bool(flag) => *flag,
            Self::Number(number) => *number != 0.0,
            Self::Text(text) => !text.is_empty(),
            Self::List(items) => !items.is_empty(),
        }
    }
}

/// `21` et non `21.0` ; `21.5` tel quel.
fn number_text(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

/// Ce qui a déclenché l'automatisation, pour `trigger.…`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriggerData {
    /// L'identifiant du déclencheur (`id:`, ou sa position).
    pub id: Option<String>,
    pub entity_id: Option<EntityId>,
    pub from_state: Option<State>,
    pub to_state: Option<State>,
    /// Pour un déclencheur `event` : son nom et ses détails
    /// (`trigger.event.event_type`, `trigger.event.data.…`).
    pub event_type: Option<String>,
    pub event_data: Option<Attributes>,
}

/// De quoi calculer un modèle.
pub(crate) struct Scope<'a> {
    pub(crate) states: &'a StateMachine,
    pub(crate) trigger: Option<&'a TriggerData>,
}

/// Le résultat d'un modèle vaut-il « vrai » ? Pour la condition `template`
/// et les détecteurs modèles, comme Home Assistant : `true`, un nombre non
/// nul, ou les textes `true`, `on`, `yes`, `open`, `home`, `1`.
#[must_use]
pub fn is_true(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => matches!(
            text.trim().to_lowercase().as_str(),
            "true" | "on" | "yes" | "open" | "home" | "1"
        ),
        Value::Null | Value::Array(_) | Value::Object(_) => false,
    }
}

impl Template {
    /// Lit un texte. `Ok(None)` s'il ne contient pas de modèle.
    ///
    /// # Errors
    /// Le modèle est mal écrit, ou utilise une fonction inconnue.
    pub fn parse(text: &str) -> Result<Option<Self>, String> {
        if text.contains("{%") || text.contains("{#") {
            return Err(
                "seuls les modèles {{ … }} sont pris en charge (pas de {% … %} ni {# … #})".into(),
            );
        }
        if !text.contains("{{") {
            // « }} » seul : presque toujours un « {{ » oublié.
            if text.contains("}}") {
                return Err("modèle : « }} » sans « {{ »".into());
            }
            return Ok(None);
        }
        if text.chars().count() > MAX_LEN {
            return Err(format!("modèle trop long ({MAX_LEN} caractères au plus)"));
        }
        let mut parts = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            if start > 0 {
                parts.push(Part::Text(rest[..start].to_owned()));
            }
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                return Err("modèle : « {{ » sans « }} »".into());
            };
            parts.push(Part::Expr(parse_expr(&after[..end])?));
            rest = &after[end + 2..];
        }
        if rest.contains("}}") {
            return Err("modèle : « }} » sans « {{ »".into());
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_owned()));
        }
        let expressions = parts.iter().filter(|p| matches!(p, Part::Expr(_))).count();
        if expressions > MAX_EXPRESSIONS {
            return Err(format!("modèle : {MAX_EXPRESSIONS} {{{{ … }}}} au plus"));
        }
        Ok(Some(Self { parts }))
    }

    /// Les entités lues par le modèle.
    pub fn entity_ids(&self) -> impl Iterator<Item = &EntityId> {
        let mut ids = Vec::new();
        for part in &self.parts {
            if let Part::Expr(node) = part {
                node.entity_ids(&mut ids);
            }
        }
        ids.into_iter()
    }

    /// Calcule le modèle hors d'une automatisation (capteurs modèles), ou
    /// avec ce qui l'a déclenchée (condition `template`).
    #[must_use]
    pub fn evaluate(&self, states: &StateMachine, trigger: Option<&TriggerData>) -> Value {
        self.render(&Scope { states, trigger })
    }

    /// Calcule le modèle. Un texte fait d'**une seule** expression donne une
    /// valeur typée (`{{ … | float }}` donne un nombre, utile pour une
    /// luminosité) ; sinon, un texte.
    pub(crate) fn render(&self, scope: &Scope<'_>) -> Value {
        if let [Part::Expr(node)] = self.parts.as_slice() {
            return match node.eval(scope) {
                Val::Number(number) => Number::from_f64(number).map_or(Value::Null, |n| {
                    // Un entier s'écrit sans « .0 » (`brightness: 128`).
                    if number.fract() == 0.0 && number.abs() < 1e15 {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "entier vérifié juste au-dessus"
                        )]
                        Value::from(number as i64)
                    } else {
                        Value::Number(n)
                    }
                }),
                Val::Bool(flag) => Value::Bool(flag),
                other => Value::String(truncate(other.text())),
            };
        }
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Expr(node) => out.push_str(&node.eval(scope).text()),
            }
            if out.len() > MAX_LEN * 4 {
                break;
            }
        }
        Value::String(truncate(out))
    }
}

fn truncate(text: String) -> String {
    if text.chars().count() <= MAX_LEN {
        return text;
    }
    text.chars().take(MAX_LEN).collect()
}

impl Node {
    fn entity_ids<'a>(&'a self, ids: &mut Vec<&'a EntityId>) {
        match self {
            Self::Base(Base::States(id) | Base::StateAttr(id, _) | Base::IsState(id, _)) => {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            Self::Base(_) => {}
            Self::List(items) => items.iter().for_each(|item| item.entity_ids(ids)),
            Self::Filter(inner, _) | Self::Neg(inner) | Self::Not(inner) => inner.entity_ids(ids),
            Self::Binary(_, left, right) => {
                left.entity_ids(ids);
                right.entity_ids(ids);
            }
            Self::If {
                condition,
                then,
                otherwise,
            } => {
                then.entity_ids(ids);
                condition.entity_ids(ids);
                otherwise.entity_ids(ids);
            }
        }
    }

    fn eval(&self, scope: &Scope<'_>) -> Val {
        match self {
            Self::Base(base) => base.eval(scope),
            Self::List(items) => Val::List(items.iter().map(|item| item.eval(scope)).collect()),
            Self::Filter(inner, filter) => filter.apply(inner.eval(scope)),
            Self::Neg(inner) => match inner.eval(scope) {
                Val::Number(number) => Val::Number(-number),
                _ => Val::None,
            },
            Self::Not(inner) => Val::Bool(!inner.eval(scope).truthy()),
            Self::If {
                condition,
                then,
                otherwise,
            } => {
                if condition.eval(scope).truthy() {
                    then.eval(scope)
                } else {
                    otherwise.eval(scope)
                }
            }
            // `and` et `or` rendent l'une des deux valeurs, comme en Python :
            // `states('x') or 'inconnu'`.
            Self::Binary(Op::And, left, right) => {
                let left = left.eval(scope);
                if left.truthy() {
                    right.eval(scope)
                } else {
                    left
                }
            }
            Self::Binary(Op::Or, left, right) => {
                let left = left.eval(scope);
                if left.truthy() {
                    left
                } else {
                    right.eval(scope)
                }
            }
            Self::Binary(op, left, right) => binary(*op, left.eval(scope), right.eval(scope)),
        }
    }
}

/// Un nombre fini, ou rien (un calcul qui déborde).
fn finite(number: f64) -> Val {
    if number.is_finite() {
        Val::Number(number)
    } else {
        Val::None
    }
}

fn binary(op: Op, left: Val, right: Val) -> Val {
    let numbers = match (&left, &right) {
        (Val::Number(a), Val::Number(b)) => Some((*a, *b)),
        _ => None,
    };
    match op {
        Op::Add => match (left, right) {
            (Val::Number(a), Val::Number(b)) => finite(a + b),
            (Val::Text(a), Val::Text(b)) => Val::Text(truncate(a + &b)),
            _ => Val::None,
        },
        Op::Sub => numbers.map_or(Val::None, |(a, b)| finite(a - b)),
        Op::Mul => numbers.map_or(Val::None, |(a, b)| finite(a * b)),
        Op::Div => match numbers {
            Some((a, b)) if b != 0.0 => finite(a / b),
            _ => Val::None,
        },
        Op::FloorDiv => match numbers {
            Some((a, b)) if b != 0.0 => finite((a / b).floor()),
            _ => Val::None,
        },
        // Le reste, du signe du diviseur, comme en Python : -7 % 3 = 2.
        Op::Mod => match numbers {
            Some((a, b)) if b != 0.0 => finite(a - b * (a / b).floor()),
            _ => Val::None,
        },
        Op::Concat => Val::Text(truncate(left.text() + &right.text())),
        Op::Eq => Val::Bool(equal(&left, &right)),
        Op::Ne => Val::Bool(!equal(&left, &right)),
        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
            let order = match (&left, &right) {
                (Val::Number(a), Val::Number(b)) => a.partial_cmp(b),
                (Val::Text(a), Val::Text(b)) => Some(a.cmp(b)),
                _ => None,
            };
            order.map_or(Val::None, |order| {
                Val::Bool(match op {
                    Op::Lt => order == Ordering::Less,
                    Op::Le => order != Ordering::Greater,
                    Op::Gt => order == Ordering::Greater,
                    _ => order != Ordering::Less,
                })
            })
        }
        Op::In => Val::Bool(contains(&right, &left)),
        Op::NotIn => Val::Bool(!contains(&right, &left)),
        // Traités dans `eval` (ils n'évaluent pas toujours les deux côtés).
        Op::And | Op::Or => Val::None,
    }
}

/// Égaux ? Un nombre n'est jamais égal à un texte : `states('x') == 21` est
/// faux, il faut `states('x') | float == 21` (comme dans Home Assistant).
#[expect(
    clippy::float_cmp,
    reason = "« == » dans un modèle : égalité exacte, comme en Jinja"
)]
fn equal(left: &Val, right: &Val) -> bool {
    match (left, right) {
        (Val::List(a), Val::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| equal(x, y))
        }
        (Val::Number(a), Val::Number(b)) => a == b,
        (Val::Text(a), Val::Text(b)) => a == b,
        (Val::Bool(a), Val::Bool(b)) => a == b,
        (Val::None, Val::None) => true,
        _ => false,
    }
}

/// `item in container` : dans une liste, ou un morceau d'un texte.
fn contains(container: &Val, item: &Val) -> bool {
    match (container, item) {
        (Val::List(items), _) => items.iter().any(|x| equal(x, item)),
        (Val::Text(text), Val::Text(part)) => text.contains(part.as_str()),
        _ => false,
    }
}

impl Base {
    fn eval(&self, scope: &Scope<'_>) -> Val {
        match self {
            Self::Literal(value) => value.clone(),
            Self::States(id) => Val::Text(
                scope
                    .states
                    .get(id)
                    .map_or_else(|| "unknown".to_owned(), |s| s.state().to_owned()),
            ),
            Self::StateAttr(id, name) => scope
                .states
                .get(id)
                .and_then(|s| s.attributes().get(name).map(Val::from_json))
                .unwrap_or(Val::None),
            Self::IsState(id, expected) => {
                Val::Bool(scope.states.get(id).is_some_and(|s| s.state() == expected))
            }
            Self::Trigger(field) => {
                let Some(trigger) = scope.trigger else {
                    return Val::None;
                };
                match field {
                    TriggerField::Id => trigger.id.clone().map_or(Val::None, Val::Text),
                    TriggerField::EntityId => trigger
                        .entity_id
                        .as_ref()
                        .map_or(Val::None, |id| Val::Text(id.to_string())),
                    TriggerField::State(side, field) => {
                        let state = match side {
                            Side::From => trigger.from_state.as_ref(),
                            Side::To => trigger.to_state.as_ref(),
                        };
                        state.map_or(Val::None, |state| state_field(state, field))
                    }
                    TriggerField::EventType => {
                        trigger.event_type.clone().map_or(Val::None, Val::Text)
                    }
                    TriggerField::EventData(name) => trigger
                        .event_data
                        .as_ref()
                        .and_then(|data| data.get(name))
                        .map_or(Val::None, Val::from_json),
                }
            }
        }
    }
}

fn state_field(state: &State, field: &StateField) -> Val {
    match field {
        StateField::State => Val::Text(state.state().to_owned()),
        StateField::EntityId => Val::Text(state.entity_id().to_string()),
        StateField::Name => Val::Text(
            state
                .attributes()
                .get("friendly_name")
                .and_then(Value::as_str)
                .map_or_else(|| state.entity_id().to_string(), str::to_owned),
        ),
        StateField::Attribute(name) => state
            .attributes()
            .get(name)
            .map_or(Val::None, Val::from_json),
    }
}

impl Filter {
    fn apply(&self, value: Val) -> Val {
        match self {
            Self::Round(digits) => value.number().map_or(Val::None, |n| {
                let factor = 10f64.powi(i32::from(*digits));
                Val::Number((n * factor).round() / factor)
            }),
            Self::Int(default) => match value.number() {
                Some(n) => Val::Number(n.trunc()),
                None => default.clone().unwrap_or(Val::None),
            },
            Self::Float(default) => match value.number() {
                Some(n) => Val::Number(n),
                None => default.clone().unwrap_or(Val::None),
            },
            Self::Abs => value.number().map_or(Val::None, |n| Val::Number(n.abs())),
            Self::Lower => text_map(value, str::to_lowercase),
            Self::Upper => text_map(value, str::to_uppercase),
            Self::Capitalize => text_map(value, |t| {
                let mut chars = t.chars();
                chars.next().map_or_else(String::new, |first| {
                    first
                        .to_uppercase()
                        .chain(chars.flat_map(char::to_lowercase))
                        .collect()
                })
            }),
            Self::Default(fallback) => match value {
                Val::None => fallback.clone(),
                other => other,
            },
            Self::Replace(from, to) => text_map(value, |t| t.replace(from.as_str(), to)),
        }
    }
}

fn text_map(value: Val, f: impl Fn(&str) -> String) -> Val {
    match value {
        Val::None => Val::None,
        other => Val::Text(truncate(f(&other.text()))),
    }
}

// ─────────────────────────────── La lecture ───────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Number(f64),
    Dot,
    Comma,
    Pipe,
    Open,
    Close,
    OpenBracket,
    CloseBracket,
    /// Un opérateur : `+`, `-`, `*`, `/`, `//`, `%`, `~`, `==`, `!=`, `<`,
    /// `<=`, `>`, `>=`.
    Symbol(&'static str),
}

/// Concept Rust : un `Peekable` permet de regarder le caractère suivant
/// sans l'avaler (`peek`), pour savoir où s'arrête un nombre ou un nom.
fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        chars.next();
        let next = chars.peek().copied();
        match c {
            ' ' | '\t' | '\n' | '\r' => {}
            '.' => tokens.push(Token::Dot),
            ',' => tokens.push(Token::Comma),
            '|' => tokens.push(Token::Pipe),
            '(' => tokens.push(Token::Open),
            ')' => tokens.push(Token::Close),
            '[' => tokens.push(Token::OpenBracket),
            ']' => tokens.push(Token::CloseBracket),
            '+' => tokens.push(Token::Symbol("+")),
            '-' => tokens.push(Token::Symbol("-")),
            '*' => tokens.push(Token::Symbol("*")),
            '%' => tokens.push(Token::Symbol("%")),
            '~' => tokens.push(Token::Symbol("~")),
            '/' if next == Some('/') => {
                chars.next();
                tokens.push(Token::Symbol("//"));
            }
            '/' => tokens.push(Token::Symbol("/")),
            '=' if next == Some('=') => {
                chars.next();
                tokens.push(Token::Symbol("=="));
            }
            '!' if next == Some('=') => {
                chars.next();
                tokens.push(Token::Symbol("!="));
            }
            '<' | '>' => {
                let with_equal = next == Some('=');
                if with_equal {
                    chars.next();
                }
                tokens.push(Token::Symbol(match (c, with_equal) {
                    ('<', false) => "<",
                    ('<', true) => "<=",
                    ('>', false) => ">",
                    _ => ">=",
                }));
            }
            '=' => return Err("modèle : « == » pour comparer (pas « = »)".into()),
            '\'' | '"' => {
                let mut value = String::new();
                loop {
                    match chars.next() {
                        Some(end) if end == c => break,
                        Some(other) => value.push(other),
                        None => return Err("modèle : texte entre guillemets non fermé".into()),
                    }
                }
                tokens.push(Token::Str(value));
            }
            '0'..='9' => {
                let mut number = String::new();
                number.push(c);
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        number.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let value = number
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| format!("modèle : nombre {number:?} invalide"))?;
                tokens.push(Token::Number(value));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut name = String::new();
                name.push(c);
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_alphanumeric() || d == '_' {
                        name.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Ident(name));
            }
            other => {
                return Err(format!("modèle : caractère {other:?} non pris en charge"));
            }
        }
    }
    Ok(tokens)
}

/// Une suite de jetons, lue de gauche à droite, par priorité : `if … else`,
/// puis `or`, `and`, `not`, les comparaisons, `+ - ~`, `* / // %`, le `-`
/// devant un nombre, les filtres, et enfin les valeurs.
struct Parser {
    tokens: Vec<Token>,
    position: usize,
    depth: usize,
    nodes: usize,
}

type Parsed = Result<Node, String>;

impl Parser {
    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.position + offset)
    }

    fn keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(name)) if name == word)
    }

    fn symbol(&self) -> Option<&'static str> {
        match self.peek() {
            Some(Token::Symbol(symbol)) => Some(symbol),
            _ => None,
        }
    }

    fn expect(&mut self, token: &Token, what: &str) -> Result<(), String> {
        match self.next() {
            Some(found) if &found == token => Ok(()),
            _ => Err(format!("modèle : {what} attendu")),
        }
    }

    fn ident(&mut self, what: &str) -> Result<String, String> {
        match self.next() {
            Some(Token::Ident(name)) => Ok(name),
            _ => Err(format!("modèle : {what} attendu")),
        }
    }

    /// Compte un morceau de plus : une expression géante est refusée.
    fn node(&mut self, node: Node) -> Parsed {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!(
                "modèle : expression trop longue ({MAX_NODES} morceaux au plus)"
            ));
        }
        Ok(node)
    }

    /// Un niveau d'imbrication de plus.
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!(
                "modèle : {MAX_DEPTH} niveaux d'imbrication au plus"
            ));
        }
        Ok(())
    }

    fn binary(&mut self, op: Op, left: Node, right: Node) -> Parsed {
        self.node(Node::Binary(op, Box::new(left), Box::new(right)))
    }

    /// `x if condition else y`.
    fn ternary(&mut self) -> Parsed {
        self.enter()?;
        let then = self.or()?;
        let node = if self.keyword("if") {
            self.next();
            let condition = self.or()?;
            if !self.keyword("else") {
                return Err("modèle : « else » attendu (x if condition else y)".into());
            }
            self.next();
            let otherwise = self.ternary()?;
            self.node(Node::If {
                condition: Box::new(condition),
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            })?
        } else {
            then
        };
        self.depth -= 1;
        Ok(node)
    }

    fn or(&mut self) -> Parsed {
        let mut left = self.and()?;
        while self.keyword("or") {
            self.next();
            let right = self.and()?;
            left = self.binary(Op::Or, left, right)?;
        }
        Ok(left)
    }

    fn and(&mut self) -> Parsed {
        let mut left = self.not()?;
        while self.keyword("and") {
            self.next();
            let right = self.not()?;
            left = self.binary(Op::And, left, right)?;
        }
        Ok(left)
    }

    fn not(&mut self) -> Parsed {
        if self.keyword("not") {
            self.next();
            self.enter()?;
            let inner = self.not()?;
            self.depth -= 1;
            return self.node(Node::Not(Box::new(inner)));
        }
        self.comparison()
    }

    /// Une seule comparaison : `a < b < c` est refusé (écrire `a < b and b < c`).
    fn comparison(&mut self) -> Parsed {
        let left = self.additive()?;
        let op = match self.symbol() {
            Some("==") => Some(Op::Eq),
            Some("!=") => Some(Op::Ne),
            Some("<") => Some(Op::Lt),
            Some("<=") => Some(Op::Le),
            Some(">") => Some(Op::Gt),
            Some(">=") => Some(Op::Ge),
            _ if self.keyword("in") => Some(Op::In),
            _ if self.keyword("not")
                && matches!(self.peek_at(1), Some(Token::Ident(name)) if name == "in") =>
            {
                self.next();
                Some(Op::NotIn)
            }
            _ => None,
        };
        let Some(op) = op else {
            return Ok(left);
        };
        self.next();
        let right = self.additive()?;
        if matches!(self.symbol(), Some("==" | "!=" | "<" | "<=" | ">" | ">="))
            || self.keyword("in")
        {
            return Err("modèle : une seule comparaison à la fois (relie-les avec and)".into());
        }
        self.binary(op, left, right)
    }

    fn additive(&mut self) -> Parsed {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.symbol() {
                Some("+") => Op::Add,
                Some("-") => Op::Sub,
                Some("~") => Op::Concat,
                _ => return Ok(left),
            };
            self.next();
            let right = self.multiplicative()?;
            left = self.binary(op, left, right)?;
        }
    }

    fn multiplicative(&mut self) -> Parsed {
        let mut left = self.unary()?;
        loop {
            let op = match self.symbol() {
                Some("*") => Op::Mul,
                Some("/") => Op::Div,
                Some("//") => Op::FloorDiv,
                Some("%") => Op::Mod,
                _ => return Ok(left),
            };
            self.next();
            let right = self.unary()?;
            left = self.binary(op, left, right)?;
        }
    }

    fn unary(&mut self) -> Parsed {
        if self.symbol() == Some("-") {
            self.next();
            self.enter()?;
            let inner = self.unary()?;
            self.depth -= 1;
            // `-3` : un nombre, pas une opération.
            if let Node::Base(Base::Literal(Val::Number(number))) = inner {
                return Ok(Node::Base(Base::Literal(Val::Number(-number))));
            }
            return self.node(Node::Neg(Box::new(inner)));
        }
        self.filtered()
    }

    /// Une valeur suivie de ses filtres : `states('x') | float | round(1)`.
    fn filtered(&mut self) -> Parsed {
        let mut node = self.primary()?;
        let mut count = 0;
        while self.peek() == Some(&Token::Pipe) {
            self.next();
            let name = self.ident("nom de filtre")?;
            let args = if self.peek() == Some(&Token::Open) {
                self.next();
                self.arguments()?
            } else {
                Vec::new()
            };
            count += 1;
            if count > MAX_FILTERS {
                return Err(format!("modèle : {MAX_FILTERS} filtres au plus"));
            }
            node = self.node(Node::Filter(Box::new(node), parse_filter(&name, args)?))?;
        }
        Ok(node)
    }

    fn primary(&mut self) -> Parsed {
        let node = match self.next() {
            Some(Token::Str(text)) => Node::Base(Base::Literal(Val::Text(text))),
            Some(Token::Number(number)) => Node::Base(Base::Literal(Val::Number(number))),
            Some(Token::Open) => {
                let inner = self.ternary()?;
                self.expect(&Token::Close, "« ) »")?;
                return Ok(inner);
            }
            Some(Token::OpenBracket) => {
                let mut items = Vec::new();
                if self.peek() == Some(&Token::CloseBracket) {
                    self.next();
                } else {
                    loop {
                        items.push(self.ternary()?);
                        if items.len() > MAX_LIST {
                            return Err(format!(
                                "modèle : {MAX_LIST} éléments au plus dans une liste"
                            ));
                        }
                        match self.next() {
                            Some(Token::Comma) => {}
                            Some(Token::CloseBracket) => break,
                            _ => return Err("modèle : « , » ou « ] » attendu".into()),
                        }
                    }
                }
                Node::List(items)
            }
            Some(Token::Ident(name)) if name == "trigger" => Node::Base(parse_trigger(self)?),
            Some(Token::Ident(name)) => {
                if let Some(value) = literal(&name) {
                    Node::Base(Base::Literal(value))
                } else {
                    Node::Base(self.function(&name)?)
                }
            }
            _ => {
                return Err(
                    "modèle : valeur attendue (states(…), state_attr(…), trigger.…, un nombre, \
                     un texte entre guillemets)"
                        .into(),
                );
            }
        };
        self.node(node)
    }

    /// `states('…')`, `state_attr('…', '…')`, `is_state('…', '…')`.
    fn function(&mut self, name: &str) -> Result<Base, String> {
        self.expect(&Token::Open, &format!("« ( » après {name}"))?;
        let args = self.arguments()?;
        let count = |n: usize| {
            if args.len() == n {
                Ok(())
            } else {
                Err(format!("{name} : {n} argument(s) attendu(s)"))
            }
        };
        match name {
            "states" => {
                count(1)?;
                Ok(Base::States(entity_arg(args.first(), "states")?))
            }
            "state_attr" => {
                count(2)?;
                Ok(Base::StateAttr(
                    entity_arg(args.first(), "state_attr")?,
                    text_arg(args.get(1), "state_attr")?,
                ))
            }
            "is_state" => {
                count(2)?;
                Ok(Base::IsState(
                    entity_arg(args.first(), "is_state")?,
                    text_arg(args.get(1), "is_state")?,
                ))
            }
            other => Err(format!(
                "modèle : fonction {other} inconnue (states, state_attr, is_state)"
            )),
        }
    }

    /// Les arguments entre parenthèses (la `(` est déjà lue) : des textes et
    /// des nombres seulement.
    fn arguments(&mut self) -> Result<Vec<Val>, String> {
        let mut values = Vec::new();
        if self.peek() == Some(&Token::Close) {
            self.next();
            return Ok(values);
        }
        loop {
            values.push(match self.next() {
                Some(Token::Str(text)) => Val::Text(text),
                Some(Token::Number(number)) => Val::Number(number),
                Some(Token::Symbol("-")) => match self.next() {
                    Some(Token::Number(number)) => Val::Number(-number),
                    _ => return Err("modèle : nombre attendu après « - »".into()),
                },
                Some(Token::Ident(name)) => literal(&name).ok_or_else(|| {
                    format!(
                        "modèle : {name} : les arguments sont des textes entre guillemets ou des nombres"
                    )
                })?,
                _ => return Err("modèle : argument attendu".into()),
            });
            match self.next() {
                Some(Token::Comma) => {}
                Some(Token::Close) => return Ok(values),
                _ => return Err("modèle : « , » ou « ) » attendu".into()),
            }
            if values.len() > 3 {
                return Err("modèle : trop d'arguments".into());
            }
        }
    }
}

fn literal(name: &str) -> Option<Val> {
    match name {
        "true" | "True" => Some(Val::Bool(true)),
        "false" | "False" => Some(Val::Bool(false)),
        "none" | "None" => Some(Val::None),
        _ => None,
    }
}

fn parse_expr(text: &str) -> Parsed {
    let mut parser = Parser {
        tokens: tokenize(text)?,
        position: 0,
        depth: 0,
        nodes: 0,
    };
    if parser.tokens.is_empty() {
        return Err("modèle : {{ }} vide".into());
    }
    let node = parser.ternary()?;
    if parser.peek().is_some() {
        return Err("modèle : « | » (filtre), une opération ou « }} » attendu".into());
    }
    Ok(node)
}

fn entity_arg(value: Option<&Val>, function: &str) -> Result<EntityId, String> {
    match value {
        Some(Val::Text(id)) => EntityId::new(id).map_err(|e| format!("{function} : {e}")),
        _ => Err(format!(
            "{function} : l'entité s'écrit entre guillemets, par exemple '{function}(\"sensor.salon\")'"
        )),
    }
}

fn text_arg(value: Option<&Val>, what: &str) -> Result<String, String> {
    match value {
        Some(Val::Text(text)) => Ok(text.clone()),
        _ => Err(format!("{what} : texte entre guillemets attendu")),
    }
}

/// `trigger.id`, `trigger.entity_id`, `trigger.to_state.state`,
/// `trigger.from_state.name`, `trigger.to_state.attributes.temperature`,
/// `trigger.to_state.attributes['Vent violent']`, `trigger.event.data.nom`.
fn parse_trigger(parser: &mut Parser) -> Result<Base, String> {
    const HELP: &str = "trigger.id, trigger.entity_id, trigger.to_state.state, \
                        trigger.from_state.state, trigger.to_state.name, \
                        trigger.to_state.attributes.…, trigger.event.data.…";
    parser.expect(&Token::Dot, "« . » après trigger")?;
    let field = match parser.ident("champ de trigger")?.as_str() {
        "id" => TriggerField::Id,
        "entity_id" => TriggerField::EntityId,
        side @ ("to_state" | "from_state") => {
            let side = if side == "to_state" {
                Side::To
            } else {
                Side::From
            };
            parser.expect(&Token::Dot, &format!("champ après trigger.{HELP}"))?;
            let field = match parser.ident("champ de l'état")?.as_str() {
                "state" => StateField::State,
                "name" => StateField::Name,
                "entity_id" => StateField::EntityId,
                "attributes" => StateField::Attribute(member(parser, "attributes")?),
                other => return Err(format!("modèle : champ {other} inconnu ({HELP})")),
            };
            TriggerField::State(side, field)
        }
        "event" => {
            parser.expect(&Token::Dot, "« . » après trigger.event")?;
            match parser.ident("champ de trigger.event")?.as_str() {
                "event_type" => TriggerField::EventType,
                "data" => TriggerField::EventData(member(parser, "data")?),
                other => {
                    return Err(format!(
                        "modèle : trigger.event.{other} inconnu (trigger.event.event_type, \
                         trigger.event.data.…)"
                    ));
                }
            }
        }
        other => return Err(format!("modèle : trigger.{other} inconnu ({HELP})")),
    };
    Ok(Base::Trigger(field))
}

/// `.nom` ou `['nom']`, après `attributes` ou `data`.
fn member(parser: &mut Parser, after: &str) -> Result<String, String> {
    match parser.next() {
        Some(Token::Dot) => parser.ident("nom"),
        Some(Token::OpenBracket) => {
            let Some(Token::Str(name)) = parser.next() else {
                return Err(format!("modèle : {after}['nom'] attendu"));
            };
            parser.expect(&Token::CloseBracket, "« ] »")?;
            Ok(name)
        }
        _ => Err(format!("modèle : {after}.nom attendu")),
    }
}

fn parse_filter(name: &str, args: Vec<Val>) -> Result<Filter, String> {
    let at_most = |n: usize| {
        if args.len() <= n {
            Ok(())
        } else {
            Err(format!("filtre {name} : {n} argument(s) au plus"))
        }
    };
    Ok(match name {
        "round" => {
            at_most(1)?;
            let digits = match args.first() {
                None => 0.0,
                Some(Val::Number(n)) => *n,
                Some(_) => return Err("filtre round : un nombre de décimales".into()),
            };
            if !(0.0..=6.0).contains(&digits) || digits.fract() != 0.0 {
                return Err("filtre round : 0 à 6 décimales".into());
            }
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "entier de 0 à 6, vérifié juste au-dessus"
            )]
            Filter::Round(digits as u8)
        }
        "int" => {
            at_most(1)?;
            Filter::Int(args.into_iter().next())
        }
        "float" => {
            at_most(1)?;
            Filter::Float(args.into_iter().next())
        }
        "abs" | "lower" | "upper" | "capitalize" => {
            at_most(0)?;
            match name {
                "abs" => Filter::Abs,
                "lower" => Filter::Lower,
                "upper" => Filter::Upper,
                _ => Filter::Capitalize,
            }
        }
        "default" => {
            if args.len() != 1 {
                return Err("filtre default : une valeur, par exemple default('?')".into());
            }
            Filter::Default(args.into_iter().next().unwrap_or(Val::None))
        }
        "replace" => {
            let [Val::Text(from), Val::Text(to)] = args.as_slice() else {
                return Err("filtre replace : deux textes, par exemple replace('_', ' ')".into());
            };
            if from.is_empty() {
                return Err("filtre replace : le texte à remplacer est vide".into());
            }
            Filter::Replace(from.clone(), to.clone())
        }
        other => {
            return Err(format!(
                "modèle : filtre {other} inconnu (round, int, float, abs, lower, upper, \
                 capitalize, default, replace)"
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::Core;
    use serde_json::json;

    fn set(states: &StateMachine, id: &str, state: &str, attributes: Value) {
        let Value::Object(map) = attributes else {
            panic!()
        };
        states.set(id.parse().unwrap(), state, map).unwrap();
    }

    fn machine() -> Core {
        let core = Core::new();
        let states = &core.states;
        set(
            states,
            "sensor.meteo_temperature",
            "21.456",
            json!({"friendly_name": "Température extérieure", "unit_of_measurement": "°C"}),
        );
        set(
            states,
            "sensor.vigilance_meteo",
            "Orange",
            json!({"friendly_name": "Vigilance météo", "Vent violent": "Jaune", "niveau_actuel": "Jaune"}),
        );
        set(states, "light.salon", "on", json!({"brightness": 128}));
        core
    }

    fn render(text: &str, states: &StateMachine, trigger: Option<&TriggerData>) -> Value {
        Template::parse(text)
            .unwrap()
            .expect("un modèle")
            .render(&Scope { states, trigger })
    }

    #[test]
    fn plain_text_is_not_a_template() {
        assert_eq!(Template::parse("Bonjour").unwrap(), None);
        assert_eq!(Template::parse("").unwrap(), None);
    }

    #[test]
    fn states_and_attributes_are_read() {
        let core = machine();
        let states = &core.states;
        assert_eq!(
            render(
                "Il fait {{ states('sensor.meteo_temperature') | round(1) }} °C.",
                states,
                None
            ),
            "Il fait 21.5 °C."
        );
        assert_eq!(
            render(
                "{{ state_attr('sensor.vigilance_meteo', 'niveau_actuel') | lower }}",
                states,
                None
            ),
            "jaune"
        );
        assert_eq!(
            render("{{ is_state('light.salon', \"on\") }}", states, None),
            true
        );
        // Une seule expression qui donne un nombre : un nombre.
        assert_eq!(
            render(
                "{{ state_attr('light.salon', 'brightness') }}",
                states,
                None
            ),
            128
        );
        assert_eq!(
            render(
                "{{ states('sensor.meteo_temperature') | float }}",
                states,
                None
            ),
            21.456
        );
        assert_eq!(
            render(
                "{{ states('sensor.meteo_temperature') | int }}",
                states,
                None
            ),
            21
        );
    }

    #[test]
    fn missing_values_give_empty_text_or_the_default() {
        let core = machine();
        let states = &core.states;
        assert_eq!(
            render("[{{ state_attr('light.salon', 'couleur') }}]", states, None),
            "[]"
        );
        assert_eq!(
            render(
                "{{ state_attr('light.cuisine', 'brightness') | default('?') }}",
                states,
                None
            ),
            "?"
        );
        assert_eq!(
            render("{{ states('light.cuisine') }}", states, None),
            "unknown"
        );
        assert_eq!(
            render("{{ states('light.cuisine') | float(0) }}", states, None),
            0
        );
        assert_eq!(
            render("{{ states('light.cuisine') | round(1) }}", states, None),
            ""
        );
        // Sans déclencheur (un script) : rien.
        assert_eq!(render("{{ trigger.to_state.state }}", states, None), "");
    }

    #[test]
    fn the_trigger_is_available() {
        let core = machine();
        let states = &core.states;
        let id: EntityId = "sensor.vigilance_meteo".parse().unwrap();
        let after = states.get(&id);
        let other = Core::new();
        set(
            &other.states,
            "sensor.vigilance_meteo",
            "Jaune",
            json!({"Vent violent": "Vert"}),
        );
        let trigger = TriggerData {
            id: Some("orange".into()),
            entity_id: Some(id.clone()),
            from_state: other.states.get(&id),
            to_state: after,
            ..TriggerData::default()
        };
        let text = "{{ trigger.to_state.name }} : {{ trigger.from_state.state | lower }} → \
                    {{ trigger.to_state.state | lower }} (vent : \
                    {{ trigger.to_state.attributes['Vent violent'] | lower }}, avant \
                    {{ trigger.from_state.attributes['Vent violent'] }}) [{{ trigger.id }}]";
        assert_eq!(
            render(text, states, Some(&trigger)),
            "Vigilance météo : jaune → orange (vent : jaune, avant Vert) [orange]"
        );
        assert_eq!(
            render(
                "{{ trigger.entity_id | replace('_', ' ') | capitalize }}",
                states,
                Some(&trigger)
            ),
            "Sensor.vigilance meteo"
        );
    }

    #[test]
    fn mistakes_are_refused_when_loading() {
        for (bad, hint) in [
            ("{{ states('sensor.x') ", "}}"),
            ("x }}", "{{"),
            ("{% if true %}oui{% endif %}", "{% … %}"),
            ("{{ }}", "vide"),
            ("{{ states(sensor.x) }}", "guillemets"),
            ("{{ states('pas une entité') }}", "states"),
            ("{{ now() }}", "fonction now inconnue"),
            ("{{ states('sensor.x') | json }}", "filtre json inconnu"),
            ("{{ states('sensor.x') @ 2 }}", "caractère '@'"),
            ("{{ states('sensor.x') = 2 }}", "=="),
            ("{{ 1 < 2 < 3 }}", "une seule comparaison"),
            ("{{ 1 if true }}", "else"),
            ("{{ (1 + 2 }}", "« ) »"),
            ("{{ [1, 2 }}", "« ] »"),
            (
                "{{ trigger.event.payload }}",
                "trigger.event.payload inconnu",
            ),
            ("{{ trigger.platform }}", "trigger.platform inconnu"),
            ("{{ trigger.to_state.color }}", "champ color inconnu"),
            ("{{ states('sensor.x') | round(9) }}", "0 à 6"),
            ("{{ 'a' | default }}", "default"),
            ("{{ states('sensor.x') states('sensor.y') }}", "filtre"),
        ] {
            let error = Template::parse(bad).unwrap_err();
            assert!(error.contains(hint), "{bad} : {error}");
        }
        let many = "{{ 1 }}".repeat(MAX_EXPRESSIONS + 1);
        assert!(Template::parse(&many).is_err());
        let long = format!("{{{{ 1 }}}}{}", "x".repeat(MAX_LEN));
        assert!(Template::parse(&long).is_err());
    }

    #[test]
    fn templates_can_calculate_and_compare() {
        let core = machine();
        let states = &core.states;
        set(states, "sensor.dehors", "8.5", json!({}));
        set(
            states,
            "zone.home",
            "2",
            json!({"persons": ["david", "alice"]}),
        );
        let r = |text: &str| render(text, states, None);
        // Calculs : les filtres passent avant les opérations, comme en Jinja.
        assert_eq!(
            r(
                "{{ (states('sensor.meteo_temperature') | float - states('sensor.dehors') | float) | round(1) }}"
            ),
            13.0
        );
        assert_eq!(r("{{ 1 + 2 * 3 }}"), 7);
        assert_eq!(r("{{ (1 + 2) * 3 }}"), 9);
        assert_eq!(r("{{ 7 / 2 }}"), 3.5);
        assert_eq!(r("{{ 7 // 2 }}"), 3);
        assert_eq!(r("{{ -7 % 3 }}"), 2, "le reste comme en Python");
        assert_eq!(
            r("{{ -(2 + 3) | abs }}"),
            -5,
            "le filtre passe avant le moins"
        );
        assert_eq!(r("{{ 'Il fait ' ~ 21 ~ ' °C' }}"), "Il fait 21 °C");
        assert_eq!(r("{{ 'a' + 'b' }}"), "ab");
        // Un calcul impossible : rien, ou la valeur par défaut.
        assert_eq!(r("{{ 1 / 0 }}"), "");
        assert_eq!(r("{{ (1 / 0) | default(0) }}"), 0);
        assert_eq!(
            r("{{ states('sensor.dehors') + 1 }}"),
            "",
            "un texte, sans | float"
        );
        // Comparaisons, et/ou/non, si/sinon.
        assert_eq!(r("{{ states('sensor.dehors') | float < 10 }}"), true);
        assert_eq!(r("{{ states('sensor.dehors') == '8.5' }}"), true);
        assert_eq!(
            r("{{ states('sensor.dehors') == 8.5 }}"),
            false,
            "texte et nombre"
        );
        assert_eq!(
            r("{{ is_state('light.salon', 'on') and states('sensor.dehors') | float < 10 }}"),
            true
        );
        assert_eq!(r("{{ not is_state('light.salon', 'on') or false }}"), false);
        assert_eq!(
            r("{{ state_attr('light.salon', 'couleur') or 'blanc' }}"),
            "blanc"
        );
        assert_eq!(
            r("{{ 'froid' if states('sensor.dehors') | float < 10 else 'doux' }}"),
            "froid"
        );
        assert_eq!(r("{{ 'a' if false else 'b' if true else 'c' }}"), "b");
        // `in` : dans une liste (écrite, ou un attribut), ou un texte.
        assert_eq!(r("{{ states('light.salon') in ['on', 'off'] }}"), true);
        assert_eq!(
            r("{{ 'david' in state_attr('zone.home', 'persons') }}"),
            true
        );
        assert_eq!(
            r("{{ 'paul' not in state_attr('zone.home', 'persons') }}"),
            true
        );
        assert_eq!(r("{{ 'Vent' in 'Vent violent' }}"), true);
        assert_eq!(
            r("{{ state_attr('zone.home', 'persons') }}"),
            "['david', 'alice']"
        );
        // Les entités lues partout dans l'expression.
        let template = Template::parse(
            "{{ states('sensor.a') | float + (1 if is_state('light.b', 'on') else state_attr('sensor.c', 'x') | float) }}",
        )
        .unwrap()
        .unwrap();
        let ids: Vec<&str> = template.entity_ids().map(EntityId::as_str).collect();
        assert_eq!(ids, ["sensor.a", "light.b", "sensor.c"]);
    }

    #[test]
    fn the_event_that_triggered_is_available() {
        let core = machine();
        let mut data = ha_core::Attributes::new();
        data.insert("action".into(), "coupe".into());
        data.insert("mesure".into(), 33.into());
        let trigger = TriggerData {
            event_type: Some("delestage".into()),
            event_data: Some(data),
            ..TriggerData::default()
        };
        assert_eq!(
            render(
                "{{ trigger.event.event_type }} : {{ trigger.event.data.action }} à {{ trigger.event.data['mesure'] }} A",
                &core.states,
                Some(&trigger)
            ),
            "delestage : coupe à 33 A"
        );
        assert_eq!(
            render(
                "{{ trigger.event.data.mesure > 30 }}",
                &core.states,
                Some(&trigger)
            ),
            true
        );
    }

    #[test]
    fn huge_or_deep_expressions_are_refused() {
        let deep = format!(
            "{{{{ {}1{} }}}}",
            "(".repeat(MAX_DEPTH + 1),
            ")".repeat(MAX_DEPTH + 1)
        );
        assert!(Template::parse(&deep).unwrap_err().contains("imbrication"));
        let nots = format!("{{{{ {}true }}}}", "not ".repeat(MAX_DEPTH + 1));
        assert!(Template::parse(&nots).unwrap_err().contains("imbrication"));
        let long = format!("{{{{ 1{} }}}}", " + 1".repeat(MAX_NODES));
        assert!(Template::parse(&long).unwrap_err().contains("trop longue"));
        let list = format!("{{{{ [{}] }}}}", vec!["1"; MAX_LIST + 1].join(", "));
        assert!(Template::parse(&list).is_err());
    }

    #[test]
    fn true_or_false_like_home_assistant() {
        for yes in [
            json!(true),
            json!(1),
            json!(2.5),
            json!("on"),
            json!("True"),
            json!(" yes "),
        ] {
            assert!(is_true(&yes), "{yes}");
        }
        for no in [
            json!(false),
            json!(0),
            json!("off"),
            json!(""),
            json!("unknown"),
            Value::Null,
        ] {
            assert!(!is_true(&no), "{no}");
        }
    }

    #[test]
    fn the_entities_read_are_known() {
        let template =
            Template::parse("{{ states('sensor.a') }} {{ state_attr('light.b', 'x') }} {{ 1 }}")
                .unwrap()
                .unwrap();
        let ids: Vec<&str> = template.entity_ids().map(EntityId::as_str).collect();
        assert_eq!(ids, ["sensor.a", "light.b"]);
    }

    #[test]
    fn the_result_stays_short() {
        let core = machine();
        let states = &core.states;
        let text = format!(
            "{}{{{{ states('light.salon') }}}}",
            "a".repeat(MAX_LEN - 40)
        );
        let Value::String(out) = render(&text, states, None) else {
            panic!()
        };
        assert!(out.chars().count() <= MAX_LEN);
    }
}
