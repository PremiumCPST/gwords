//! Special characters palette: categories, search, recently used.

use std::cell::{OnceCell, RefCell};
use std::collections::HashSet;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use self::Category::*;

/// Categories of the static table (the « Récents » entry is dynamic).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
    Typographie,
    Symboles,
    Fleches,
    Maths,
    Grec,
    Devises,
    Fractions,
    Accentuees,
}

impl Category {
    pub(crate) const ALL: [Category; 8] = [
        Typographie,
        Symboles,
        Fleches,
        Maths,
        Grec,
        Devises,
        Fractions,
        Accentuees,
    ];

    fn label(self) -> &'static str {
        match self {
            Typographie => "Typographie",
            Symboles => "Symboles",
            Fleches => "Flèches",
            Maths => "Maths",
            Grec => "Grec",
            Devises => "Devises",
            Fractions => "Fractions et indices",
            Accentuees => "Lettres accentuées",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Typographie => "typo",
            Symboles => "symboles",
            Fleches => "fleches",
            Maths => "maths",
            Grec => "grec",
            Devises => "devises",
            Fractions => "fractions",
            Accentuees => "accents",
        }
    }
}

/// Static table: (character, French name, category).
/// Non-breaking spaces are inserted as real spaces; they are displayed as "⍽".
pub(crate) const CHARS: &[(&str, &str, Category)] = &[
    // Typographie
    ("«", "Guillemet ouvrant", Typographie),
    ("»", "Guillemet fermant", Typographie),
    ("‹", "Guillemet simple ouvrant", Typographie),
    ("›", "Guillemet simple fermant", Typographie),
    ("“", "Guillemet double ouvrant", Typographie),
    ("”", "Guillemet double fermant", Typographie),
    ("‘", "Apostrophe ouvrante", Typographie),
    ("’", "Apostrophe fermante", Typographie),
    ("„", "Guillemet bas double", Typographie),
    ("‚", "Guillemet bas simple", Typographie),
    ("—", "Tiret cadratin", Typographie),
    ("–", "Tiret demi-cadratin", Typographie),
    ("…", "Points de suspension", Typographie),
    ("·", "Point médian", Typographie),
    ("•", "Puce", Typographie),
    ("¶", "Pied de mouche", Typographie),
    ("§", "Paragraphe", Typographie),
    ("†", "Croix (renvoi)", Typographie),
    ("‡", "Double croix (renvoi)", Typographie),
    ("‰", "Pour mille", Typographie),
    ("′", "Prime", Typographie),
    ("″", "Seconde prime", Typographie),
    ("¡", "Point d'exclamation inversé", Typographie),
    ("¿", "Point d'interrogation inversé", Typographie),
    ("©", "Copyright", Typographie),
    ("®", "Marque déposée", Typographie),
    ("™", "Marque de commerce", Typographie),
    ("°", "Degré", Typographie),
    ("№", "Numéro", Typographie),
    ("★", "Étoile pleine", Typographie),
    ("☆", "Étoile vide", Typographie),
    ("✓", "Coche", Typographie),
    ("✗", "Croix (ballot)", Typographie),
    ("♥", "Cœur", Typographie),
    ("\u{a0}", "Espace insécable", Typographie),
    ("\u{202f}", "Espace fine insécable", Typographie),
    ("\u{2009}", "Espace fine", Typographie),
    ("\u{2002}", "Espace demi-cadratin", Typographie),
    ("\u{2003}", "Espace cadratin", Typographie),
    ("\u{2011}", "Trait d'union insécable", Typographie),
    // Symboles
    ("@", "Arobase", Symboles),
    ("#", "Dièse", Symboles),
    ("&", "Esperluette", Symboles),
    ("*", "Astérisque", Symboles),
    ("/", "Barre oblique", Symboles),
    ("\\", "Barre oblique inverse", Symboles),
    ("|", "Barre verticale", Symboles),
    ("¦", "Barre verticale brisée", Symboles),
    ("~", "Tilde", Symboles),
    ("^", "Accent circonflexe", Symboles),
    ("`", "Accent grave", Symboles),
    ("¨", "Tréma", Symboles),
    ("´", "Accent aigu", Symboles),
    ("¸", "Cédille", Symboles),
    ("¯", "Macron", Symboles),
    ("ª", "Ordinal féminin", Symboles),
    ("º", "Ordinal masculin", Symboles),
    ("♠", "Pique", Symboles),
    ("♣", "Trèfle", Symboles),
    ("♦", "Carreau", Symboles),
    ("♪", "Note de musique", Symboles),
    ("♫", "Notes de musique", Symboles),
    ("☐", "Case vide", Symboles),
    ("☑", "Case cochée", Symboles),
    ("☒", "Case cochée avec croix", Symboles),
    ("☎", "Téléphone", Symboles),
    ("✉", "Enveloppe", Symboles),
    ("⌘", "Touche Commande", Symboles),
    ("⌥", "Touche Option", Symboles),
    ("⇧", "Touche Majuscule", Symboles),
    ("⌫", "Retour arrière", Symboles),
    ("⏎", "Touche Entrée", Symboles),
    ("⚠", "Avertissement", Symboles),
    ("☀", "Soleil", Symboles),
    ("☁", "Nuage", Symboles),
    ("☂", "Parapluie", Symboles),
    ("✂", "Ciseaux", Symboles),
    ("✈", "Avion", Symboles),
    ("⚑", "Drapeau", Symboles),
    ("☯", "Yin et yang", Symboles),
    ("♻", "Recyclage", Symboles),
    ("⌂", "Maison", Symboles),
    // Flèches
    ("←", "Flèche gauche", Fleches),
    ("→", "Flèche droite", Fleches),
    ("↑", "Flèche haut", Fleches),
    ("↓", "Flèche bas", Fleches),
    ("↔", "Flèche gauche droite", Fleches),
    ("↕", "Flèche haut bas", Fleches),
    ("↖", "Flèche diagonale haut gauche", Fleches),
    ("↗", "Flèche diagonale haut droite", Fleches),
    ("↘", "Flèche diagonale bas droite", Fleches),
    ("↙", "Flèche diagonale bas gauche", Fleches),
    ("⇐", "Double flèche gauche", Fleches),
    ("⇒", "Double flèche droite", Fleches),
    ("⇑", "Double flèche haut", Fleches),
    ("⇓", "Double flèche bas", Fleches),
    ("⇔", "Double flèche gauche droite", Fleches),
    ("↩", "Flèche retour gauche", Fleches),
    ("↪", "Flèche retour droite", Fleches),
    ("↵", "Flèche retour à la ligne", Fleches),
    ("⟵", "Longue flèche gauche", Fleches),
    ("⟶", "Longue flèche droite", Fleches),
    ("⟷", "Longue flèche gauche droite", Fleches),
    ("⟸", "Longue double flèche gauche", Fleches),
    ("⟹", "Longue double flèche droite", Fleches),
    ("↦", "Flèche à barre droite", Fleches),
    ("➔", "Flèche droite épaisse", Fleches),
    ("➜", "Flèche droite arrondie", Fleches),
    ("⤴", "Flèche courbe haut", Fleches),
    ("⤵", "Flèche courbe bas", Fleches),
    ("↻", "Flèche circulaire horaire", Fleches),
    ("↺", "Flèche circulaire antihoraire", Fleches),
    // Maths
    ("+", "Plus", Maths),
    ("−", "Moins", Maths),
    ("±", "Plus ou moins", Maths),
    ("×", "Multiplié par", Maths),
    ("÷", "Divisé par", Maths),
    ("⋅", "Produit scalaire (point)", Maths),
    ("≠", "Différent de", Maths),
    ("≈", "Environ égal à", Maths),
    ("≃", "Asymptotiquement égal à", Maths),
    ("≅", "Isomorphe à", Maths),
    ("∼", "Semblable à", Maths),
    ("≡", "Identique à", Maths),
    ("≤", "Inférieur ou égal à", Maths),
    ("≥", "Supérieur ou égal à", Maths),
    ("≪", "Très inférieur à", Maths),
    ("≫", "Très supérieur à", Maths),
    ("∞", "Infini", Maths),
    ("√", "Racine carrée", Maths),
    ("∛", "Racine cubique", Maths),
    ("∑", "Somme", Maths),
    ("∏", "Produit", Maths),
    ("∫", "Intégrale", Maths),
    ("∬", "Intégrale double", Maths),
    ("∮", "Intégrale curviligne", Maths),
    ("∂", "Dérivée partielle", Maths),
    ("∆", "Delta (variation)", Maths),
    ("∇", "Nabla", Maths),
    ("∈", "Appartient à", Maths),
    ("∉", "N'appartient pas à", Maths),
    ("∋", "Contient (élément)", Maths),
    ("⊂", "Inclus dans", Maths),
    ("⊃", "Contient (inclusion)", Maths),
    ("⊆", "Inclus ou égal à", Maths),
    ("⊇", "Contient ou égal à", Maths),
    ("∪", "Union", Maths),
    ("∩", "Intersection", Maths),
    ("∀", "Pour tout", Maths),
    ("∃", "Il existe", Maths),
    ("∄", "Il n'existe pas", Maths),
    ("¬", "Négation logique", Maths),
    ("∧", "Et logique", Maths),
    ("∨", "Ou logique", Maths),
    ("⊕", "Ou exclusif", Maths),
    ("⊗", "Produit tensoriel", Maths),
    ("∝", "Proportionnel à", Maths),
    ("∅", "Ensemble vide", Maths),
    ("∴", "Donc", Maths),
    ("∵", "Parce que", Maths),
    ("⊥", "Perpendiculaire", Maths),
    ("∥", "Parallèle", Maths),
    ("∠", "Angle", Maths),
    ("ℕ", "Entiers naturels", Maths),
    ("ℤ", "Entiers relatifs", Maths),
    ("ℚ", "Nombres rationnels", Maths),
    ("ℝ", "Nombres réels", Maths),
    ("ℂ", "Nombres complexes", Maths),
    ("µ", "Micro", Maths),
    ("′", "Prime", Maths),
    ("″", "Seconde prime", Maths),
    ("‰", "Pour mille", Maths),
    ("°", "Degré", Maths),
    ("⌊", "Partie entière inférieure (gauche)", Maths),
    ("⌋", "Partie entière inférieure (droite)", Maths),
    ("⌈", "Partie entière supérieure (gauche)", Maths),
    ("⌉", "Partie entière supérieure (droite)", Maths),
    // Grec
    ("α", "Alpha minuscule", Grec),
    ("β", "Bêta minuscule", Grec),
    ("γ", "Gamma minuscule", Grec),
    ("δ", "Delta minuscule", Grec),
    ("ε", "Epsilon minuscule", Grec),
    ("ζ", "Zêta minuscule", Grec),
    ("η", "Êta minuscule", Grec),
    ("θ", "Thêta minuscule", Grec),
    ("ι", "Iota minuscule", Grec),
    ("κ", "Kappa minuscule", Grec),
    ("λ", "Lambda minuscule", Grec),
    ("μ", "Mu minuscule", Grec),
    ("ν", "Nu minuscule", Grec),
    ("ξ", "Xi minuscule", Grec),
    ("ο", "Omicron minuscule", Grec),
    ("π", "Pi minuscule", Grec),
    ("ρ", "Rhô minuscule", Grec),
    ("σ", "Sigma minuscule", Grec),
    ("ς", "Sigma final minuscule", Grec),
    ("τ", "Tau minuscule", Grec),
    ("υ", "Upsilon minuscule", Grec),
    ("φ", "Phi minuscule", Grec),
    ("χ", "Khi minuscule", Grec),
    ("ψ", "Psi minuscule", Grec),
    ("ω", "Oméga minuscule", Grec),
    ("Α", "Alpha majuscule", Grec),
    ("Β", "Bêta majuscule", Grec),
    ("Γ", "Gamma majuscule", Grec),
    ("Δ", "Delta majuscule", Grec),
    ("Ε", "Epsilon majuscule", Grec),
    ("Ζ", "Zêta majuscule", Grec),
    ("Η", "Êta majuscule", Grec),
    ("Θ", "Thêta majuscule", Grec),
    ("Ι", "Iota majuscule", Grec),
    ("Κ", "Kappa majuscule", Grec),
    ("Λ", "Lambda majuscule", Grec),
    ("Μ", "Mu majuscule", Grec),
    ("Ν", "Nu majuscule", Grec),
    ("Ξ", "Xi majuscule", Grec),
    ("Ο", "Omicron majuscule", Grec),
    ("Π", "Pi majuscule", Grec),
    ("Ρ", "Rhô majuscule", Grec),
    ("Σ", "Sigma majuscule", Grec),
    ("Τ", "Tau majuscule", Grec),
    ("Υ", "Upsilon majuscule", Grec),
    ("Φ", "Phi majuscule", Grec),
    ("Χ", "Khi majuscule", Grec),
    ("Ψ", "Psi majuscule", Grec),
    ("Ω", "Oméga majuscule", Grec),
    // Devises
    ("$", "Dollar", Devises),
    ("€", "Euro", Devises),
    ("£", "Livre sterling", Devises),
    ("¥", "Yen", Devises),
    ("¢", "Cent", Devises),
    ("¤", "Devise générique", Devises),
    ("₹", "Roupie indienne", Devises),
    ("₽", "Rouble", Devises),
    ("₩", "Won", Devises),
    ("₺", "Livre turque", Devises),
    ("₿", "Bitcoin", Devises),
    ("₣", "Franc", Devises),
    ("₱", "Peso", Devises),
    ("₴", "Hryvnia", Devises),
    ("₦", "Naira", Devises),
    ("₫", "Dong", Devises),
    ("₪", "Shekel", Devises),
    ("₡", "Colón", Devises),
    ("₲", "Guaraní", Devises),
    ("₵", "Cedi", Devises),
    ("₭", "Kip", Devises),
    ("₮", "Tugrik", Devises),
    ("₠", "Ecu", Devises),
    ("₥", "Mill", Devises),
    ("₨", "Roupie", Devises),
    // Fractions et indices
    ("½", "Un demi", Fractions),
    ("⅓", "Un tiers", Fractions),
    ("⅔", "Deux tiers", Fractions),
    ("¼", "Un quart", Fractions),
    ("¾", "Trois quarts", Fractions),
    ("⅕", "Un cinquième", Fractions),
    ("⅙", "Un sixième", Fractions),
    ("⅛", "Un huitième", Fractions),
    ("⅜", "Trois huitièmes", Fractions),
    ("⅝", "Cinq huitièmes", Fractions),
    ("⅞", "Sept huitièmes", Fractions),
    ("⁄", "Barre de fraction", Fractions),
    ("⁰", "Exposant zéro", Fractions),
    ("¹", "Exposant un", Fractions),
    ("²", "Exposant deux (carré)", Fractions),
    ("³", "Exposant trois (cube)", Fractions),
    ("⁴", "Exposant quatre", Fractions),
    ("⁵", "Exposant cinq", Fractions),
    ("⁶", "Exposant six", Fractions),
    ("⁷", "Exposant sept", Fractions),
    ("⁸", "Exposant huit", Fractions),
    ("⁹", "Exposant neuf", Fractions),
    ("⁺", "Exposant plus", Fractions),
    ("⁻", "Exposant moins", Fractions),
    ("⁼", "Exposant égal", Fractions),
    ("⁽", "Exposant parenthèse ouvrante", Fractions),
    ("⁾", "Exposant parenthèse fermante", Fractions),
    ("ⁿ", "Exposant n", Fractions),
    ("ⁱ", "Exposant i", Fractions),
    ("₀", "Indice zéro", Fractions),
    ("₁", "Indice un", Fractions),
    ("₂", "Indice deux", Fractions),
    ("₃", "Indice trois", Fractions),
    ("₄", "Indice quatre", Fractions),
    ("₅", "Indice cinq", Fractions),
    ("₆", "Indice six", Fractions),
    ("₇", "Indice sept", Fractions),
    ("₈", "Indice huit", Fractions),
    ("₉", "Indice neuf", Fractions),
    ("₊", "Indice plus", Fractions),
    ("₋", "Indice moins", Fractions),
    ("₌", "Indice égal", Fractions),
    ("₍", "Indice parenthèse ouvrante", Fractions),
    ("₎", "Indice parenthèse fermante", Fractions),
    ("ₐ", "Indice a", Fractions),
    ("ₑ", "Indice e", Fractions),
    ("ₒ", "Indice o", Fractions),
    ("ₓ", "Indice x", Fractions),
    ("ₕ", "Indice h", Fractions),
    ("ₖ", "Indice k", Fractions),
    ("ₗ", "Indice l", Fractions),
    ("ₘ", "Indice m", Fractions),
    ("ₙ", "Indice n", Fractions),
    ("ₚ", "Indice p", Fractions),
    ("ₛ", "Indice s", Fractions),
    ("ₜ", "Indice t", Fractions),
    // Lettres accentuées
    ("À", "A majuscule accent grave", Accentuees),
    ("Â", "A majuscule accent circonflexe", Accentuees),
    ("Ä", "A majuscule tréma", Accentuees),
    ("Á", "A majuscule accent aigu", Accentuees),
    ("Ã", "A majuscule tilde", Accentuees),
    ("Å", "A majuscule anneau", Accentuees),
    ("Æ", "Ligature AE majuscule", Accentuees),
    ("Ç", "C majuscule cédille", Accentuees),
    ("É", "E majuscule accent aigu", Accentuees),
    ("È", "E majuscule accent grave", Accentuees),
    ("Ê", "E majuscule accent circonflexe", Accentuees),
    ("Ë", "E majuscule tréma", Accentuees),
    ("Í", "I majuscule accent aigu", Accentuees),
    ("Î", "I majuscule accent circonflexe", Accentuees),
    ("Ï", "I majuscule tréma", Accentuees),
    ("Ñ", "N majuscule tilde", Accentuees),
    ("Ó", "O majuscule accent aigu", Accentuees),
    ("Ô", "O majuscule accent circonflexe", Accentuees),
    ("Ö", "O majuscule tréma", Accentuees),
    ("Õ", "O majuscule tilde", Accentuees),
    ("Ø", "O majuscule barré", Accentuees),
    ("Œ", "Ligature OE majuscule", Accentuees),
    ("Ú", "U majuscule accent aigu", Accentuees),
    ("Ù", "U majuscule accent grave", Accentuees),
    ("Û", "U majuscule accent circonflexe", Accentuees),
    ("Ü", "U majuscule tréma", Accentuees),
    ("Ÿ", "Y majuscule tréma", Accentuees),
    ("à", "a minuscule accent grave", Accentuees),
    ("â", "a minuscule accent circonflexe", Accentuees),
    ("ä", "a minuscule tréma", Accentuees),
    ("á", "a minuscule accent aigu", Accentuees),
    ("ã", "a minuscule tilde", Accentuees),
    ("å", "a minuscule anneau", Accentuees),
    ("æ", "Ligature ae minuscule", Accentuees),
    ("ç", "c minuscule cédille", Accentuees),
    ("é", "e minuscule accent aigu", Accentuees),
    ("è", "e minuscule accent grave", Accentuees),
    ("ê", "e minuscule accent circonflexe", Accentuees),
    ("ë", "e minuscule tréma", Accentuees),
    ("í", "i minuscule accent aigu", Accentuees),
    ("ì", "i minuscule accent grave", Accentuees),
    ("î", "i minuscule accent circonflexe", Accentuees),
    ("ï", "i minuscule tréma", Accentuees),
    ("ñ", "n minuscule tilde", Accentuees),
    ("ó", "o minuscule accent aigu", Accentuees),
    ("ò", "o minuscule accent grave", Accentuees),
    ("ô", "o minuscule accent circonflexe", Accentuees),
    ("ö", "o minuscule tréma", Accentuees),
    ("õ", "o minuscule tilde", Accentuees),
    ("ø", "o minuscule barré", Accentuees),
    ("œ", "Ligature oe minuscule", Accentuees),
    ("ú", "u minuscule accent aigu", Accentuees),
    ("ù", "u minuscule accent grave", Accentuees),
    ("û", "u minuscule accent circonflexe", Accentuees),
    ("ü", "u minuscule tréma", Accentuees),
    ("ÿ", "y minuscule tréma", Accentuees),
    ("ß", "Eszett (s dur)", Accentuees),
];

/// Number of recently used characters kept in memory.
const MAX_RECENTS: usize = 24;

thread_local! {
    static RECENTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn push_recent(ch: &str) {
    RECENTS.with_borrow_mut(|recents| {
        recents.retain(|c| c != ch);
        recents.insert(0, ch.to_owned());
        recents.truncate(MAX_RECENTS);
    });
}

fn recents() -> Vec<String> {
    RECENTS.with_borrow(|recents| recents.clone())
}

/// Lowercases and removes accents (é -> e, œ -> oe, ...) for accent-insensitive search.
pub(crate) fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        let replacement = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a",
            'ç' => "c",
            'è' | 'é' | 'ê' | 'ë' => "e",
            'ì' | 'í' | 'î' | 'ï' => "i",
            'ñ' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => "o",
            'ù' | 'ú' | 'û' | 'ü' => "u",
            'ý' | 'ÿ' => "y",
            'œ' => "oe",
            'æ' => "ae",
            'ß' => "ss",
            _ => "",
        };
        if replacement.is_empty() {
            out.push(c);
        } else {
            out.push_str(replacement);
        }
    }
    out
}

/// Characters of one category, in table order.
fn entries_of(category: Category) -> Vec<(&'static str, &'static str)> {
    CHARS
        .iter()
        .filter(|(_, _, cat)| *cat == category)
        .map(|(ch, name, _)| (*ch, *name))
        .collect()
}

/// Characters matching every word of `query` (folded, in the name or the character itself).
/// Each character is returned once, even when it appears in several categories.
pub(crate) fn search(query: &str) -> Vec<(&'static str, &'static str)> {
    let words: Vec<String> = fold(query).split_whitespace().map(str::to_owned).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut hits = Vec::new();
    for (ch, name, _) in CHARS {
        let haystack = format!("{} {}", fold(name), ch);
        if words.iter().all(|w| haystack.contains(w.as_str())) && seen.insert(*ch) {
            hits.push((*ch, *name));
        }
    }
    hits
}

/// Visible label of a character: invisible spaces are shown as "⍽".
fn display_label(ch: &str) -> &str {
    match ch {
        "\u{a0}" | "\u{202f}" | "\u{2009}" | "\u{2002}" | "\u{2003}" | "\u{2011}" => "⍽",
        _ => ch,
    }
}

/// French name of a character (the character itself when it is not in the table).
fn name_of(ch: &str) -> &str {
    CHARS
        .iter()
        .find(|(c, _, _)| *c == ch)
        .map_or(ch, |(_, name, _)| *name)
}

const CATEGORY_LABEL_RECENT: &str = "Récents";

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SpecialCharsPopover {
        pub(super) ui: OnceCell<Ui>,
        pub(super) callbacks: RefCell<Vec<Box<dyn Fn(&str)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpecialCharsPopover {
        const NAME: &'static str = "GwSpecialCharsPopover";
        type Type = super::SpecialCharsPopover;
        type ParentType = gtk::Popover;
    }

    impl ObjectImpl for SpecialCharsPopover {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build_ui();
        }
    }
    impl WidgetImpl for SpecialCharsPopover {}
    impl PopoverImpl for SpecialCharsPopover {}
}

glib::wrapper! {
    pub struct SpecialCharsPopover(ObjectSubclass<imp::SpecialCharsPopover>)
        @extends gtk::Popover, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::ShortcutManager;
}

/// A scrollable grid of character buttons with an empty-state message.
#[derive(Clone)]
struct GridPage {
    root: gtk::Box,
    flow: gtk::FlowBox,
    message: gtk::Label,
}

impl GridPage {
    fn new(empty_message: &str) -> Self {
        let flow = gtk::FlowBox::new();
        flow.set_homogeneous(true);
        flow.set_max_children_per_line(10);
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_column_spacing(4);
        flow.set_row_spacing(4);
        flow.set_valign(gtk::Align::Start);

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&flow));

        let message = gtk::Label::new(Some(empty_message));
        message.add_css_class("dim-label");
        message.set_margin_top(24);
        message.set_visible(false);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&message);
        root.append(&scroller);

        Self {
            root,
            flow,
            message,
        }
    }
}

/// Widgets of the popover, created once in `constructed`.
struct Ui {
    search: gtk::SearchEntry,
    dropdown: gtk::DropDown,
    stack: gtk::Stack,
    recent: GridPage,
    results: GridPage,
}

impl Default for SpecialCharsPopover {
    fn default() -> Self {
        Self::new()
    }
}

impl SpecialCharsPopover {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Called with the chosen character(s) (the popover stays open so several
    /// characters can be inserted; Escape closes it).
    /// Callbacks must not register new callbacks while being called.
    pub fn connect_char_selected<F: Fn(&str) + 'static>(&self, f: F) {
        self.imp().callbacks.borrow_mut().push(Box::new(f));
    }

    fn ui(&self) -> &Ui {
        self.imp()
            .ui
            .get()
            .expect("special characters UI is built in constructed()")
    }

    fn build_ui(&self) {
        self.add_css_class("special-chars");

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher un caractère"));
        search.set_hexpand(true);

        let mut labels = vec![CATEGORY_LABEL_RECENT];
        labels.extend(Category::ALL.iter().map(|c| c.label()));
        let dropdown = gtk::DropDown::from_strings(&labels);
        dropdown.set_halign(gtk::Align::Start);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_transition_duration(200);
        stack.set_vexpand(true);

        let recent = GridPage::new("Aucun caractère récent");
        stack.add_named(&recent.root, Some("recent"));
        for category in Category::ALL {
            let page = GridPage::new("Aucun caractère");
            self.fill(&page, &entries_of(category));
            stack.add_named(&page.root, Some(category.key()));
        }
        let results = GridPage::new("Aucun caractère trouvé");
        stack.add_named(&results.root, Some("results"));

        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(12);
        root.set_margin_end(12);
        root.set_size_request(380, 340);
        root.append(&search);
        root.append(&dropdown);
        root.append(&stack);
        self.set_child(Some(&root));

        let weak = self.downgrade();
        search.connect_search_changed(move |entry| {
            if let Some(obj) = weak.upgrade() {
                obj.on_search(&entry.text());
            }
        });

        let weak = self.downgrade();
        dropdown.connect_selected_notify(move |_| {
            if let Some(obj) = weak.upgrade() {
                obj.on_category_changed();
            }
        });

        let weak = self.downgrade();
        self.connect_map(move |_| {
            if let Some(obj) = weak.upgrade() {
                obj.refresh_recent();
                // Nothing used yet: open on « Typographie » rather than an empty page.
                let ui = obj.ui();
                if ui.dropdown.selected() == 0 && recents().is_empty() {
                    ui.dropdown.set_selected(1);
                }
                ui.search.grab_focus();
            }
        });

        let ui = Ui {
            search,
            dropdown,
            stack,
            recent,
            results,
        };
        if self.imp().ui.set(ui).is_err() {
            unreachable!("build_ui runs once per popover");
        }
    }

    /// Replaces the buttons of a page with `entries` (character, French name).
    fn fill(&self, page: &GridPage, entries: &[(&str, &str)]) {
        while let Some(child) = page.flow.first_child() {
            page.flow.remove(&child);
        }
        for (ch, name) in entries {
            let button = gtk::Button::with_label(display_label(ch));
            button.add_css_class("flat");
            button.add_css_class("special-char");
            button.set_tooltip_text(Some(name));

            let weak = self.downgrade();
            let ch = ch.to_string();
            button.connect_clicked(move |_| {
                if let Some(obj) = weak.upgrade() {
                    obj.insert_char(&ch);
                }
            });
            page.flow.append(&button);
        }
        page.message.set_visible(entries.is_empty());
    }

    fn refresh_recent(&self) {
        let ui = self.ui();
        let chars = recents();
        let entries: Vec<(&str, &str)> = chars.iter().map(|c| (c.as_str(), name_of(c))).collect();
        self.fill(&ui.recent, &entries);
    }

    /// Stack page shown for the dropdown index (0 = « Récents »).
    fn page_name(index: u32) -> &'static str {
        match index {
            0 => "recent",
            i => Category::ALL
                .get(i as usize - 1)
                .map_or("recent", |c| c.key()),
        }
    }

    fn on_category_changed(&self) {
        let ui = self.ui();
        if !ui.search.text().is_empty() {
            return;
        }
        let index = ui.dropdown.selected();
        if index == 0 {
            self.refresh_recent();
        }
        ui.stack.set_visible_child_name(Self::page_name(index));
    }

    fn on_search(&self, query: &str) {
        let ui = self.ui();
        if query.trim().is_empty() {
            ui.dropdown.set_sensitive(true);
            self.on_category_changed();
            return;
        }
        ui.dropdown.set_sensitive(false);
        let hits = search(query);
        self.fill(&ui.results, &hits);
        ui.stack.set_visible_child_name("results");
    }

    /// Records the character as recently used and notifies the callbacks.
    fn insert_char(&self, ch: &str) {
        push_recent(ch);
        for callback in self.imp().callbacks.borrow().iter() {
            callback(ch);
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn table_entries_are_non_empty() {
        for (ch, name, _) in CHARS {
            assert!(!ch.is_empty(), "empty character for {name:?}");
            assert!(!name.is_empty(), "empty name for {ch:?}");
        }
    }

    #[test]
    fn no_duplicate_chars_within_a_category() {
        for category in Category::ALL {
            let mut seen = HashSet::new();
            for (ch, _, cat) in CHARS {
                if *cat == category {
                    assert!(seen.insert(*ch), "duplicate {ch:?} in {category:?}");
                }
            }
        }
    }

    #[test]
    fn greek_entries_are_greek() {
        for (ch, name, cat) in CHARS {
            if *cat == Grec {
                let c = ch.chars().next().unwrap();
                assert!(('\u{370}'..='\u{3ff}').contains(&c), "{name} is not Greek");
            }
        }
    }

    #[test]
    fn fold_removes_accents_and_case() {
        assert_eq!(fold("Flèche"), "fleche");
        assert_eq!(fold("ÉCOLE"), "ecole");
        assert_eq!(fold("Cœur"), "coeur");
        assert_eq!(fold("Straße"), "strasse");
    }

    #[test]
    fn search_is_accent_and_case_insensitive() {
        let hits = search("fleche");
        assert!(hits.iter().any(|(ch, _)| *ch == "→"));
        assert!(search("FLÈCHE").iter().any(|(ch, _)| *ch == "→"));
        assert!(search("coeur").iter().any(|(ch, _)| *ch == "♥"));
    }

    #[test]
    fn search_matches_the_character_itself() {
        assert!(search("€").iter().any(|(ch, _)| *ch == "€"));
    }

    #[test]
    fn search_requires_every_word_and_returns_unique_chars() {
        assert!(search("zzz-no-match").is_empty());
        assert!(search("   ").is_empty());
        let hits = search("degré");
        let mut chars: Vec<_> = hits.iter().map(|(c, _)| *c).collect();
        let before = chars.len();
        chars.sort_unstable();
        chars.dedup();
        assert_eq!(before, chars.len());
    }

    #[test]
    fn page_names_match_categories() {
        assert_eq!(SpecialCharsPopover::page_name(0), "recent");
        assert_eq!(SpecialCharsPopover::page_name(1), Typographie.key());
        assert_eq!(SpecialCharsPopover::page_name(8), Accentuees.key());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// GTK-dependent checks, called from `crate::gtk_tests`.
    pub(crate) fn gtk_checks() {
        let popover = SpecialCharsPopover::new();
        let received = std::rc::Rc::new(RefCell::new(Vec::<String>::new()));
        let sink = received.clone();
        popover.connect_char_selected(move |c| sink.borrow_mut().push(c.to_owned()));

        popover.on_search("fleche");
        let ui = popover.ui();
        assert_eq!(ui.stack.visible_child_name().as_deref(), Some("results"));
        assert!(ui.results.flow.first_child().is_some());
        assert!(!ui.dropdown.is_sensitive());

        popover.on_search("");
        assert!(ui.dropdown.is_sensitive());
        assert_eq!(ui.stack.visible_child_name().as_deref(), Some("recent"));

        popover.insert_char("→");
        assert_eq!(received.borrow().as_slice(), ["→"]);
        assert_eq!(recents().first().map(String::as_str), Some("→"));
    }
}
