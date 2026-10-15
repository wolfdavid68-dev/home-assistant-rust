//! Les **modèles** : des valeurs dans les textes des actions.
//!
//! ```yaml
//! - action: notify.send_message
//!   target: {entity_id: notify.telephone}
//!   data:
//!     message: "Il fait {{ states('sensor.meteo_temperature') }} °C dehors."
//! ```
//!
//! On reprend l'écriture de Home Assistant (Jinja), mais seulement une petite
//! partie, faite pour **lire** : l'état d'une entité, un attribut, ce qui a
//! déclenché l'automatisation, et quelques filtres (`round`, `float`,
//! `default`…). Pas de boucles, pas de calculs, pas d'accès aux fichiers :
//! un modèle ne peut rien faire d'autre que produire un texte court.
//!
//! Chaque modèle est vérifié au chargement (fonctions et filtres connus,
//! identifiants d'entités valides). À l'exécution, une valeur introuvable
//! donne un texte vide, ou la valeur de `default(…)` : jamais d'erreur qui
//! bloque l'automatisation.

use ha_core::{EntityId, State, StateMachine};
use serde_json::{Number, Value};

/// Longueur maximale d'un texte qui contient un modèle, et du résultat.
const MAX_LEN: usize = 2000;
/// Nombre maximal de `{{ … }}` dans un texte.
const MAX_EXPRESSIONS: usize = 20;
/// Nombre maximal de filtres dans une expression.
const MAX_FILTERS: usize = 5;

/// Un texte avec des `{{ … }}`, vérifié.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(String),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
struct Expr {
    base: Base,
    filters: Vec<Filter>,
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
    Lower,
    Upper,
    Capitalize,
    Default(Val),
    Replace(String, String),
}

/// Une valeur calculée.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    /// Introuvable (entité ou attribut absent, nombre illisible…).
    None,
    Text(String),
    Number(f64),
    Bool(bool),
}

impl Val {
    fn from_json(value: &Value) -> Self {
        match value {
            Value::String(text) => Self::Text(text.clone()),
            Value::Number(number) => number.as_f64().map_or(Self::None, Self::Number),
            Value::Bool(flag) => Self::Bool(*flag),
            // Listes et objets : trop gros pour un message.
            Value::Null | Value::Array(_) | Value::Object(_) => Self::None,
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::Text(text) => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
            Self::Bool(_) | Self::None => None,
        }
    }

    fn text(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Text(text) => text.clone(),
            Self::Number(number) => number_text(*number),
            Self::Bool(true) => "True".into(),
            Self::Bool(false) => "False".into(),
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
}

/// De quoi calculer un modèle.
pub(crate) struct Scope<'a> {
    pub(crate) states: &'a StateMachine,
    pub(crate) trigger: Option<&'a TriggerData>,
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
        self.parts.iter().filter_map(|part| match part {
            Part::Expr(Expr {
                base: Base::States(id) | Base::StateAttr(id, _) | Base::IsState(id, _),
                ..
            }) => Some(id),
            _ => None,
        })
    }

    /// Calcule le modèle. Un texte fait d'**une seule** expression donne une
    /// valeur typée (`{{ … | float }}` donne un nombre, utile pour une
    /// luminosité) ; sinon, un texte.
    pub(crate) fn render(&self, scope: &Scope<'_>) -> Value {
        if let [Part::Expr(expr)] = self.parts.as_slice() {
            return match expr.eval(scope) {
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
                Part::Expr(expr) => out.push_str(&expr.eval(scope).text()),
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

impl Expr {
    fn eval(&self, scope: &Scope<'_>) -> Val {
        let mut value = self.base.eval(scope);
        for filter in &self.filters {
            value = filter.apply(value);
        }
        value
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
        other => Val::Text(f(&other.text())),
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
}

/// Concept Rust : un `Peekable` permet de regarder le caractère suivant
/// sans l'avaler (`peek`), pour savoir où s'arrête un nombre ou un nom.
fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }
            '.' => {
                chars.next();
                tokens.push(Token::Dot);
            }
            ',' => {
                chars.next();
                tokens.push(Token::Comma);
            }
            '|' => {
                chars.next();
                tokens.push(Token::Pipe);
            }
            '(' => {
                chars.next();
                tokens.push(Token::Open);
            }
            ')' => {
                chars.next();
                tokens.push(Token::Close);
            }
            '[' => {
                chars.next();
                tokens.push(Token::OpenBracket);
            }
            ']' => {
                chars.next();
                tokens.push(Token::CloseBracket);
            }
            '\'' | '"' => {
                chars.next();
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
            '-' | '0'..='9' => {
                let mut number = String::new();
                number.push(c);
                chars.next();
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
                return Err(format!(
                    "modèle : caractère {other:?} non pris en charge (pas de calculs ni de \
                     comparaisons, seulement des fonctions et des filtres)"
                ));
            }
        }
    }
    Ok(tokens)
}

/// Une suite de jetons, lue de gauche à droite.
struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
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

fn parse_expr(text: &str) -> Result<Expr, String> {
    let mut parser = Parser {
        tokens: tokenize(text)?,
        position: 0,
    };
    if parser.tokens.is_empty() {
        return Err("modèle : {{ }} vide".into());
    }
    let base = parse_base(&mut parser)?;
    let mut filters = Vec::new();
    while let Some(token) = parser.next() {
        if token != Token::Pipe {
            return Err("modèle : « | » (filtre) ou « }} » attendu".into());
        }
        let name = parser.ident("nom de filtre")?;
        let args = if parser.peek() == Some(&Token::Open) {
            parser.next();
            parser.arguments()?
        } else {
            Vec::new()
        };
        filters.push(parse_filter(&name, args)?);
        if filters.len() > MAX_FILTERS {
            return Err(format!("modèle : {MAX_FILTERS} filtres au plus"));
        }
    }
    Ok(Expr { base, filters })
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

fn parse_base(parser: &mut Parser) -> Result<Base, String> {
    match parser.next() {
        Some(Token::Str(text)) => Ok(Base::Literal(Val::Text(text))),
        Some(Token::Number(number)) => Ok(Base::Literal(Val::Number(number))),
        Some(Token::Ident(name)) if name == "trigger" => parse_trigger(parser),
        Some(Token::Ident(name)) => {
            if let Some(value) = literal(&name) {
                return Ok(Base::Literal(value));
            }
            parser.expect(&Token::Open, &format!("« ( » après {name}"))?;
            let args = parser.arguments()?;
            let count = |n: usize| {
                if args.len() == n {
                    Ok(())
                } else {
                    Err(format!("{name} : {n} argument(s) attendu(s)"))
                }
            };
            match name.as_str() {
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
        _ => Err("modèle : valeur attendue (states(…), state_attr(…), trigger.…)".into()),
    }
}

/// `trigger.id`, `trigger.entity_id`, `trigger.to_state.state`,
/// `trigger.from_state.name`, `trigger.to_state.attributes.temperature`,
/// `trigger.to_state.attributes['Vent violent']`.
fn parse_trigger(parser: &mut Parser) -> Result<Base, String> {
    const HELP: &str = "trigger.id, trigger.entity_id, trigger.to_state.state, \
                        trigger.from_state.state, trigger.to_state.name ou \
                        trigger.to_state.attributes.…";
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
                "attributes" => StateField::Attribute(match parser.next() {
                    Some(Token::Dot) => parser.ident("nom de l'attribut")?,
                    Some(Token::OpenBracket) => {
                        let Some(Token::Str(name)) = parser.next() else {
                            return Err("modèle : attributes['nom'] attendu".into());
                        };
                        parser.expect(&Token::CloseBracket, "« ] »")?;
                        name
                    }
                    _ => return Err("modèle : attributes.nom attendu".into()),
                }),
                other => return Err(format!("modèle : champ {other} inconnu ({HELP})")),
            };
            TriggerField::State(side, field)
        }
        other => return Err(format!("modèle : trigger.{other} inconnu ({HELP})")),
    };
    Ok(Base::Trigger(field))
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
        "lower" | "upper" | "capitalize" => {
            at_most(0)?;
            match name {
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
                "modèle : filtre {other} inconnu (round, int, float, lower, upper, \
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
            ("{{ states('sensor.x') * 2 }}", "calculs"),
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
