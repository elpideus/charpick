//! Character catalogue: language and symbol sections, readable names and search.
//!
//! Each section lists its characters as a space separated string. An entry may be a
//! single character, a short digraph (`l·l`), or an explicit case pair `lower|UPPER`
//! for letters whose uppercase is not the default Unicode mapping (Turkish `i|İ`).

use std::collections::HashMap;

use crate::gfx::Icon;
use crate::util::{fold, nfd};

pub struct SectionDef {
    pub id: &'static str,
    /// Label shown in the navigation bar.
    pub tab: &'static str,
    /// Title in the section's own language (or a descriptive title for symbols).
    pub native: &'static str,
    /// English name, shown next to the native title.
    pub english: &'static str,
    /// Extra search keywords.
    pub aliases: &'static str,
    pub chars: &'static str,
    pub is_lang: bool,
}

const fn lang(
    id: &'static str,
    tab: &'static str,
    native: &'static str,
    english: &'static str,
    aliases: &'static str,
    chars: &'static str,
) -> SectionDef {
    SectionDef { id, tab, native, english, aliases, chars, is_lang: true }
}

const fn sym(
    id: &'static str,
    tab: &'static str,
    title: &'static str,
    aliases: &'static str,
    chars: &'static str,
) -> SectionDef {
    SectionDef { id, tab, native: title, english: "", aliases, chars, is_lang: false }
}

/// Default order: Italian first, Romanian second, then the big western European
/// languages, then everything else, then symbols.
pub static SECTIONS: &[SectionDef] = &[
    lang("it", "IT", "Italiano", "Italian", "italia ita", "à è é ì ò ù í î ó ú « » “ ” ’ ª º"),
    lang("ro", "RO", "Română", "Romanian", "romania moldovan moldova rum ron", "ă â î ș ț „ ” « » ş ţ"),
    lang("de", "DE", "Deutsch", "German", "germany austria swiss ger deu", "ä ö ü ß „ “ ‚ ‘ » « › ‹"),
    lang(
        "fr",
        "FR",
        "Français",
        "French",
        "france belgian quebec fra fre",
        "à â æ ç é è ê ë î ï ô œ ù û ü ÿ « » ‹ › “ ” ’",
    ),
    lang("es", "ES", "Español", "Spanish", "castellano spain mexico latin spa", "á é í ó ú ñ ü ¿ ¡ « » “ ” ª º"),
    lang(
        "pt",
        "PT",
        "Português",
        "Portuguese",
        "portugal brazil brasil brazilian por",
        "á â ã à ç é ê í ó ô õ ú « » “ ” ª º",
    ),
    lang("ca", "CA", "Català", "Catalan", "valencian valencia cat", "à è é í ï ò ó ú ü ç l·l · « »"),
    lang("nl", "NL", "Nederlands", "Dutch", "flemish netherlands holland vlaams nld dut", "á é í ó ú à è ë ï ö ü ĳ"),
    lang("sv", "SV", "Svenska", "Swedish", "sweden swe", "å ä ö é ” »"),
    lang("no", "NO", "Norsk", "Norwegian", "norway bokmal bokmål nynorsk nb nn nor", "æ ø å é è ê ó ò ô « »"),
    lang("da", "DA", "Dansk", "Danish", "denmark dan", "æ ø å é » «"),
    lang("fi", "FI", "Suomi", "Finnish", "finland fin", "ä ö å š ž ”"),
    lang("is", "IS", "Íslenska", "Icelandic", "iceland faroese isl", "á ð é í ó ú ý þ æ ö „ “"),
    lang("pl", "PL", "Polski", "Polish", "poland pol", "ą ć ę ł ń ó ś ź ż „ ” « »"),
    lang("cs", "CS", "Čeština", "Czech", "czechia cesky ces cze cz", "á č ď é ě í ň ó ř š ť ú ů ý ž „ “ ‚ ‘"),
    lang("sk", "SK", "Slovenčina", "Slovak", "slovakia slovensky slk", "á ä č ď é í ĺ ľ ň ó ô ŕ š ť ú ý ž „ “"),
    lang("hu", "HU", "Magyar", "Hungarian", "hungary hun", "á é í ó ö ő ú ü ű „ ” » «"),
    lang(
        "hr",
        "HR",
        "Hrvatski · Bosanski · Srpski",
        "Croatian, Bosnian, Serbian",
        "croatia bosnia serbia montenegrin crnogorski latinica sr bs hrv srp bos",
        "č ć đ š ž „ “ » «",
    ),
    lang("sl", "SL", "Slovenščina", "Slovenian", "slovene slovenia slv", "č š ž ć đ „ “"),
    lang("tr", "TR", "Türkçe", "Turkish", "turkey turkiye azerbaijani azeri tur", "ç ğ ı i|İ ö ş ü â î û"),
    lang("et", "ET", "Eesti", "Estonian", "estonia est", "ä ö õ ü š ž „ “"),
    lang("lv", "LV", "Latviešu", "Latvian", "latvia lettish lav", "ā č ē ģ ī ķ ļ ņ š ū ž"),
    lang("lt", "LT", "Lietuvių", "Lithuanian", "lithuania lit", "ą č ę ė į š ų ū ž „ “"),
    lang("sq", "SQ", "Shqip", "Albanian", "albania kosovo sqi alb", "ç ë « »"),
    lang("mt", "MT", "Malti", "Maltese", "malta mlt", "ċ ġ għ ħ ż à è ì ò ù"),
    lang("cy", "CY", "Cymraeg", "Welsh", "wales cym", "â ê î ô û ŵ ŷ á é í ó ú ẃ ý ä ë ï ö ü ẅ ÿ"),
    lang("ga", "GA", "Gaeilge", "Irish", "ireland gaelic scottish gle", "á é í ó ú à è ì ò ù"),
    lang("eo", "EO", "Esperanto", "Esperanto", "epo", "ĉ ĝ ĥ ĵ ŝ ŭ"),
    lang(
        "vi",
        "VI",
        "Tiếng Việt",
        "Vietnamese",
        "vietnam vie",
        "ă â đ ê ô ơ ư \
         à á ả ã ạ ằ ắ ẳ ẵ ặ ầ ấ ẩ ẫ ậ \
         è é ẻ ẽ ẹ ề ế ể ễ ệ \
         ì í ỉ ĩ ị \
         ò ó ỏ õ ọ ồ ố ổ ỗ ộ ờ ớ ở ỡ ợ \
         ù ú ủ ũ ụ ừ ứ ử ữ ự \
         ỳ ý ỷ ỹ ỵ",
    ),
    lang(
        "el",
        "EL",
        "Ελληνικά",
        "Greek",
        "greece hellenic ellinika ell gre",
        "α β γ δ ε ζ η θ ι κ λ μ ν ξ ο π ρ σ ς τ υ φ χ ψ ω ά έ ή ί ό ύ ώ ϊ ϋ « »",
    ),
    sym(
        "punct",
        "“”",
        "Punctuation & quotes",
        "punctuation quotes quotation marks dashes typography",
        "– — … · • « » ‹ › “ ” „ ‘ ’ ‚ ¡ ¿ ′ ″ § ¶ † ‡ ‰ № ‽",
    ),
    sym("currency", "€", "Currency", "currency money symbols", "€ £ ¥ ¢ ₿ ₹ ₽ ₺ ₴ ₩ ₪ ₫ ₱ ₦ ₸ ₼ ₾ ₡ ₵ ฿ ¤"),
    sym(
        "math",
        "±",
        "Math",
        "math maths mathematics operators logic sets",
        "± × ÷ − ≠ ≈ ≡ ≤ ≥ ∞ √ ∑ ∏ ∫ ∂ ∆ ∇ ∈ ∉ ⊂ ⊃ ∪ ∩ ∀ ∃ ∅ ¬ ∧ ∨ ⇒ ⇔ ∝ ° µ π",
    ),
    sym(
        "numbers",
        "½",
        "Fractions & superscripts",
        "numbers fractions superscripts subscripts powers indices",
        "½ ⅓ ⅔ ¼ ¾ ⅛ ⅜ ⅝ ⅞ ⁰ ¹ ² ³ ⁴ ⁵ ⁶ ⁷ ⁸ ⁹ ⁺ ⁻ ⁿ ₀ ₁ ₂ ₃ ₄ ₅ ₆ ₇ ₈ ₉",
    ),
    sym(
        "misc",
        "→",
        "Arrows & signs",
        "arrows signs misc symbols marks",
        "← → ↑ ↓ ↔ ↕ ⇐ ⇑ ⇓ ↵ ✓ ✗ ★ ☆ ♥ © ® ™ ℃ ℉ ♀ ♂ ♪ ⌘ ⌥ ⇧ ⌫",
    ),
];

/// Names for characters that do not decompose into a base letter plus marks.
/// Format: `char`, `name`, `extra keywords`.
static NAMES: &[(&str, &str, &str)] = &[
    // Letters
    ("ß", "sharp s", "eszett ss scharfes s"),
    ("æ", "ae ligature", "ae ash a"),
    ("œ", "oe ligature", "oe o"),
    ("ø", "o with stroke", "o slash"),
    ("ł", "l with stroke", "l slash"),
    ("đ", "d with stroke", "d crossed dj"),
    ("ð", "eth", "d edh"),
    ("þ", "thorn", "th t"),
    ("ı", "dotless i", "i"),
    ("ħ", "h with stroke", "h"),
    ("ĳ", "ij ligature", "ij i j"),
    ("l·l", "l geminada", "ela geminada l l middle dot"),
    ("għ", "għ digraph", "gh g h ajn"),
    ("ş", "s with cedilla (legacy, prefer ș)", "s cedilla"),
    ("ţ", "t with cedilla (legacy, prefer ț)", "t cedilla"),
    ("ș", "s with comma below", "s comma"),
    ("ț", "t with comma below", "t comma"),
    ("ς", "final sigma", "sigma s"),
    // Quotes & punctuation
    ("«", "left guillemet", "quote quotation angle double"),
    ("»", "right guillemet", "quote quotation angle double"),
    ("‹", "single left guillemet", "quote quotation angle"),
    ("›", "single right guillemet", "quote quotation angle"),
    ("“", "left double quote", "quotation curly smart"),
    ("”", "right double quote", "quotation curly smart"),
    ("„", "double low quote", "quotation low-9 opening"),
    ("‘", "left single quote", "quotation curly smart"),
    ("’", "right single quote / apostrophe", "apostrophe quotation curly"),
    ("‚", "single low quote", "quotation low-9"),
    ("–", "en dash", "dash hyphen range"),
    ("—", "em dash", "dash hyphen"),
    ("…", "ellipsis", "dots three"),
    ("·", "middle dot", "interpunct punt volat dot"),
    ("•", "bullet", "dot list"),
    ("¡", "inverted exclamation mark", "exclamation upside down"),
    ("¿", "inverted question mark", "question upside down"),
    ("′", "prime", "minutes feet apostrophe"),
    ("″", "double prime", "seconds inches"),
    ("§", "section sign", "paragraph law"),
    ("¶", "pilcrow", "paragraph"),
    ("†", "dagger", "obelisk death"),
    ("‡", "double dagger", ""),
    ("‰", "per mille", "permille thousand percent"),
    ("№", "numero sign", "number no"),
    ("‽", "interrobang", "question exclamation"),
    ("ª", "feminine ordinal indicator", "ordinal a primera"),
    ("º", "masculine ordinal indicator", "ordinal o primo degree"),
    ("°", "degree sign", "degrees temperature angle"),
    // Currency
    ("€", "euro sign", "eur money"),
    ("£", "pound sign", "sterling gbp lira"),
    ("¥", "yen sign", "yuan jpy cny"),
    ("¢", "cent sign", "cents"),
    ("₿", "bitcoin sign", "btc crypto"),
    ("₹", "indian rupee sign", "inr rupee"),
    ("₽", "ruble sign", "rub rouble"),
    ("₺", "turkish lira sign", "try lira"),
    ("₴", "hryvnia sign", "uah ukraine"),
    ("₩", "won sign", "krw korea"),
    ("₪", "new shekel sign", "ils israel"),
    ("₫", "dong sign", "vnd vietnam"),
    ("₱", "peso sign", "php philippines"),
    ("₦", "naira sign", "ngn nigeria"),
    ("₸", "tenge sign", "kzt kazakhstan"),
    ("₼", "manat sign", "azn azerbaijan"),
    ("₾", "lari sign", "gel georgia"),
    ("₡", "colon sign", "crc costa rica colón"),
    ("₵", "cedi sign", "ghs ghana"),
    ("฿", "baht sign", "thb thailand"),
    ("¤", "currency sign", "generic"),
    // Math
    ("±", "plus-minus sign", "plus minus"),
    ("×", "multiplication sign", "times multiply"),
    ("÷", "division sign", "divide obelus"),
    ("−", "minus sign", "subtract"),
    ("≠", "not equal to", "different unequal"),
    ("≈", "almost equal to", "approximately about"),
    ("≡", "identical to", "equivalent congruent"),
    ("≤", "less-than or equal to", "less equal"),
    ("≥", "greater-than or equal to", "greater equal"),
    ("∞", "infinity", "infinite"),
    ("√", "square root", "root radical"),
    ("∑", "summation", "sum sigma"),
    ("∏", "product", "pi"),
    ("∫", "integral", ""),
    ("∂", "partial differential", "derivative"),
    ("∆", "increment", "delta change triangle"),
    ("∇", "nabla", "del gradient"),
    ("∈", "element of", "in member set"),
    ("∉", "not an element of", "not in set"),
    ("⊂", "subset of", "set"),
    ("⊃", "superset of", "set"),
    ("∪", "union", "set"),
    ("∩", "intersection", "set"),
    ("∀", "for all", "universal"),
    ("∃", "there exists", "exists existential"),
    ("∅", "empty set", "null"),
    ("¬", "not sign", "negation logic"),
    ("∧", "logical and", "conjunction"),
    ("∨", "logical or", "disjunction"),
    ("⇒", "implies", "double arrow right then"),
    ("⇔", "if and only if", "iff equivalent double arrow"),
    ("∝", "proportional to", ""),
    ("µ", "micro sign", "mu micron"),
    // Numbers
    ("½", "one half", "fraction half"),
    ("⅓", "one third", "fraction"),
    ("⅔", "two thirds", "fraction"),
    ("¼", "one quarter", "fraction fourth"),
    ("¾", "three quarters", "fraction fourths"),
    ("⅛", "one eighth", "fraction"),
    ("⅜", "three eighths", "fraction"),
    ("⅝", "five eighths", "fraction"),
    ("⅞", "seven eighths", "fraction"),
    ("⁰", "superscript zero", "power exponent 0"),
    ("¹", "superscript one", "power exponent 1"),
    ("²", "superscript two", "power squared 2"),
    ("³", "superscript three", "power cubed 3"),
    ("⁴", "superscript four", "power exponent 4"),
    ("⁵", "superscript five", "power exponent 5"),
    ("⁶", "superscript six", "power exponent 6"),
    ("⁷", "superscript seven", "power exponent 7"),
    ("⁸", "superscript eight", "power exponent 8"),
    ("⁹", "superscript nine", "power exponent 9"),
    ("⁺", "superscript plus", "power"),
    ("⁻", "superscript minus", "power negative"),
    ("ⁿ", "superscript n", "power exponent"),
    ("₀", "subscript zero", "index 0"),
    ("₁", "subscript one", "index 1"),
    ("₂", "subscript two", "index 2"),
    ("₃", "subscript three", "index 3"),
    ("₄", "subscript four", "index 4"),
    ("₅", "subscript five", "index 5"),
    ("₆", "subscript six", "index 6"),
    ("₇", "subscript seven", "index 7"),
    ("₈", "subscript eight", "index 8"),
    ("₉", "subscript nine", "index 9"),
    // Arrows & signs
    ("←", "leftwards arrow", "arrow left"),
    ("→", "rightwards arrow", "arrow right"),
    ("↑", "upwards arrow", "arrow up"),
    ("↓", "downwards arrow", "arrow down"),
    ("↔", "left right arrow", "arrow both"),
    ("↕", "up down arrow", "arrow both"),
    ("⇐", "leftwards double arrow", "arrow left"),
    ("⇑", "upwards double arrow", "arrow up"),
    ("⇓", "downwards double arrow", "arrow down"),
    ("↵", "return arrow", "enter carriage newline"),
    ("✓", "check mark", "tick yes done"),
    ("✗", "ballot x", "cross no wrong"),
    ("★", "black star", "star rating"),
    ("☆", "white star", "star rating"),
    ("♥", "heart suit", "heart love"),
    ("©", "copyright sign", "copyright c"),
    ("®", "registered sign", "registered trademark r"),
    ("™", "trade mark sign", "trademark tm"),
    ("℃", "degree celsius", "celsius temperature"),
    ("℉", "degree fahrenheit", "fahrenheit temperature"),
    ("♀", "female sign", "woman venus"),
    ("♂", "male sign", "man mars"),
    ("♪", "eighth note", "music note"),
    ("⌘", "command key", "mac place of interest"),
    ("⌥", "option key", "mac alt"),
    ("⇧", "shift key", "upwards white arrow"),
    ("⌫", "erase to the left", "backspace delete"),
    ("π", "pi", "greek circle"),
];

static GREEK: &[(char, &str)] = &[
    ('α', "alpha"),
    ('β', "beta"),
    ('γ', "gamma"),
    ('δ', "delta"),
    ('ε', "epsilon"),
    ('ζ', "zeta"),
    ('η', "eta"),
    ('θ', "theta"),
    ('ι', "iota"),
    ('κ', "kappa"),
    ('λ', "lambda"),
    ('μ', "mu"),
    ('ν', "nu"),
    ('ξ', "xi"),
    ('ο', "omicron"),
    ('π', "pi"),
    ('ρ', "rho"),
    ('σ', "sigma"),
    ('τ', "tau"),
    ('υ', "upsilon"),
    ('φ', "phi"),
    ('χ', "chi"),
    ('ψ', "psi"),
    ('ω', "omega"),
];

/// Combining marks: (code point, Latin name, Greek name, extra keywords).
static MARKS: &[(u32, &str, &str, &str)] = &[
    (0x0300, "grave", "varia", "accent"),
    (0x0301, "acute", "tonos", "accent"),
    (0x0302, "circumflex", "circumflex", "hat"),
    (0x0303, "tilde", "tilde", ""),
    (0x0304, "macron", "macron", "bar long"),
    (0x0306, "breve", "breve", "short"),
    (0x0307, "dot above", "dot above", "dot"),
    (0x0308, "diaeresis", "dialytika", "umlaut trema dots"),
    (0x0309, "hook above", "hook above", "hook"),
    (0x030A, "ring above", "ring above", "ring"),
    (0x030B, "double acute", "double acute", "accent"),
    (0x030C, "caron", "caron", "hacek háček"),
    (0x031B, "horn", "horn", ""),
    (0x0323, "dot below", "dot below", "dot"),
    (0x0326, "comma below", "comma below", "comma"),
    (0x0327, "cedilla", "cedilla", "cedille"),
    (0x0328, "ogonek", "ogonek", "tail"),
];

pub struct Entry {
    pub lower: String,
    pub upper: String,
    pub name: String,
    pub upper_name: String,
    /// Folded search tokens.
    tokens: Vec<String>,
    /// Section indices (into `Catalog::sections`) of the languages that use it.
    pub langs: Vec<u16>,
}

impl Entry {
    pub fn text(&self, upper: bool) -> &str {
        if upper { &self.upper } else { &self.lower }
    }

    pub fn name(&self, upper: bool) -> &str {
        if upper && self.upper != self.lower { &self.upper_name } else { &self.name }
    }

    #[cfg(test)]
    pub fn is_cased(&self) -> bool {
        self.upper != self.lower
    }

    fn matches(&self, query: &[String]) -> bool {
        query.iter().all(|q| {
            self.tokens.iter().any(|t| if q.chars().count() == 1 { t == q } else { t.starts_with(q.as_str()) })
        })
    }
}

pub struct Section {
    pub def: &'static SectionDef,
    pub entries: Vec<u32>,
    keywords: Vec<String>,
}

impl Section {
    fn matches(&self, q: &str) -> bool {
        self.def.id == q || self.keywords.iter().any(|k| k.starts_with(q))
    }
}

pub struct Catalog {
    pub entries: Vec<Entry>,
    pub sections: Vec<Section>,
    by_text: HashMap<String, u32>,
}

/// One block of the grid as currently displayed.
pub struct ViewSection {
    pub title: String,
    pub subtitle: String,
    pub tab: String,
    /// Shown on the tab instead of the text label.
    pub tab_icon: Option<Icon>,
    pub entries: Vec<u32>,
}

impl Catalog {
    pub fn build() -> Catalog {
        let mut cat = Catalog { entries: Vec::new(), sections: Vec::new(), by_text: HashMap::new() };
        for (si, def) in SECTIONS.iter().enumerate() {
            let mut ids = Vec::new();
            for token in def.chars.split_whitespace() {
                let (lower, upper) = match token.split_once('|') {
                    Some((l, u)) => (l.to_string(), Some(u.to_string())),
                    None => (token.to_string(), None),
                };
                let id = match cat.by_text.get(&lower) {
                    Some(&id) => id,
                    None => {
                        let id = cat.entries.len() as u32;
                        cat.entries.push(make_entry(&lower, upper));
                        cat.by_text.insert(lower, id);
                        id
                    }
                };
                if def.is_lang && !cat.entries[id as usize].langs.contains(&(si as u16)) {
                    cat.entries[id as usize].langs.push(si as u16);
                }
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            let mut keywords: Vec<String> = Vec::new();
            for src in [def.native, def.english, def.aliases] {
                keywords
                    .extend(fold(src).split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(String::from));
            }
            cat.sections.push(Section { def, entries: ids, keywords });
        }
        cat
    }

    pub fn entry_by_text(&self, s: &str) -> Option<u32> {
        self.by_text.get(s).copied()
    }

    pub fn section_index(&self, id: &str) -> Option<usize> {
        self.sections.iter().position(|s| s.def.id.eq_ignore_ascii_case(id))
    }

    /// Resolve the user's section order (`it, ro, *`) into section indices.
    pub fn resolve_order(&self, spec: &[String]) -> Vec<usize> {
        let mut order = Vec::new();
        let mut star = false;
        for s in spec {
            if s == "*" {
                star = true;
            } else if let Some(i) = self.section_index(s)
                && !order.contains(&i)
            {
                order.push(i);
            }
        }
        if star || order.is_empty() {
            for i in 0..self.sections.len() {
                if !order.contains(&i) {
                    order.push(i);
                }
            }
        }
        order
    }

    pub fn section_view(&self, i: usize) -> ViewSection {
        let def = self.sections[i].def;
        ViewSection {
            title: def.native.to_string(),
            subtitle: if def.english == def.native { String::new() } else { def.english.to_string() },
            tab: def.tab.to_string(),
            tab_icon: None,
            entries: self.sections[i].entries.clone(),
        }
    }

    /// The default view: recently used characters followed by every section.
    pub fn browse(&self, order: &[usize], recent: &[u32]) -> Vec<ViewSection> {
        let mut view = Vec::with_capacity(order.len() + 1);
        if !recent.is_empty() {
            view.push(ViewSection {
                title: "Recently used".into(),
                subtitle: String::new(),
                tab: String::new(),
                tab_icon: Some(Icon::Recent),
                entries: recent.to_vec(),
            });
        }
        view.extend(order.iter().map(|&i| self.section_view(i)));
        view
    }

    /// Search by character name (`e grave`, `s comma`), base letter (`a`), symbol
    /// keyword (`euro`) or language (`romanian`, `română`, `ro`).
    pub fn search(&self, query: &str, order: &[usize]) -> Vec<ViewSection> {
        let folded = fold(query);
        let q = folded.trim();
        let mut view = Vec::new();
        if q.is_empty() {
            return view;
        }
        let tokens: Vec<String> = q.split_whitespace().map(String::from).collect();
        let rank: HashMap<usize, usize> = order.iter().enumerate().map(|(r, &s)| (s, r)).collect();

        // Characters, ranked by the user's section order then by position in the section.
        let mut hits: Vec<(usize, usize, u32)> = Vec::new();
        for &si in order {
            for (pos, &id) in self.sections[si].entries.iter().enumerate() {
                if self.entries[id as usize].matches(&tokens) && !hits.iter().any(|h| h.2 == id) {
                    hits.push((rank[&si], pos, id));
                }
            }
        }
        hits.sort();
        if !hits.is_empty() {
            view.push(ViewSection {
                title: "Characters".into(),
                subtitle: format!("matching “{}”", query.trim()),
                tab: String::new(),
                tab_icon: Some(Icon::Search),
                entries: hits.into_iter().map(|h| h.2).collect(),
            });
        }

        // Whole sections whose language or topic matches.
        if q.chars().count() >= 2 {
            for &si in order {
                if self.sections[si].matches(q) {
                    view.push(self.section_view(si));
                }
            }
        }
        view
    }

    /// Comma separated English names of the languages that use an entry.
    pub fn languages_of(&self, id: u32) -> String {
        let langs = &self.entries[id as usize].langs;
        let names: Vec<&str> = langs.iter().map(|&s| self.sections[s as usize].def.english).collect();
        if names.len() > 4 { format!("{}, +{} more", names[..4].join(", "), names.len() - 4) } else { names.join(", ") }
    }
}

fn make_entry(lower: &str, explicit_upper: Option<String>) -> Entry {
    let upper = explicit_upper.unwrap_or_else(|| uppercase(lower));
    let (name, keywords) = describe(lower);
    let upper_name = if upper == lower { name.clone() } else { capitalize_name(&name) };

    let mut tokens: Vec<String> = Vec::new();
    for src in [name.as_str(), keywords.as_str()] {
        for w in fold(src).split(|c: char| !c.is_alphanumeric()) {
            if !w.is_empty() && w != "with" && w != "and" && !tokens.iter().any(|t| t == w) {
                tokens.push(w.to_string());
            }
        }
    }
    // The literal character, so typing `è` on an Italian keyboard finds it too.
    tokens.push(lower.to_lowercase());
    Entry { lower: lower.to_string(), upper, name, upper_name, tokens, langs: Vec::new() }
}

fn uppercase(s: &str) -> String {
    match s {
        "ß" => "ẞ".into(),
        "ς" => "Σ".into(),
        "l·l" => "L·L".into(),
        "għ" => "GĦ".into(),
        _ => {
            let up = s.to_uppercase();
            // Only accept mappings that keep the entry a single character
            // (e.g. ʼn → ʼN would be two); symbols map to themselves.
            if up.chars().count() == s.chars().count() { up } else { s.to_string() }
        }
    }
}

/// "e with grave" → "E with grave"; "sharp s" → "capital sharp s".
fn capitalize_name(name: &str) -> String {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), Some(' ')) | (Some(c), None) => {
            let mut s: String = c.to_uppercase().collect();
            s.push_str(&name[c.len_utf8()..]);
            s
        }
        _ => format!("capital {name}"),
    }
}

fn describe(s: &str) -> (String, String) {
    if let Some(&(_, name, kw)) = NAMES.iter().find(|n| n.0 == s) {
        return (name.to_string(), kw.to_string());
    }
    let mut chars = s.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return (s.to_string(), String::new());
    };
    if let Some(&(_, g)) = GREEK.iter().find(|g| g.0 == c) {
        return (g.to_string(), "greek".into());
    }
    let decomposed = nfd(s);
    let mut it = decomposed.chars();
    let Some(base) = it.next() else { return (s.to_string(), String::new()) };
    let greek = GREEK.iter().find(|g| g.0 == base).map(|g| g.1);
    let mut marks = Vec::new();
    let mut extra = Vec::new();
    for m in it {
        if let Some(&(_, latin, gk, kw)) = MARKS.iter().find(|x| x.0 == m as u32) {
            marks.push(if greek.is_some() { gk } else { latin });
            if !kw.is_empty() {
                extra.push(kw);
            }
            if greek.is_some() && gk != latin {
                extra.push(latin);
            }
        }
    }
    let base_name = match greek {
        Some(g) => g.to_string(),
        None => base.to_string(),
    };
    if marks.is_empty() {
        return (base_name, if greek.is_some() { "greek".into() } else { String::new() });
    }
    if greek.is_some() {
        extra.push("greek");
    }
    (format!("{base_name} with {}", marks.join(" and ")), extra.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_case() {
        let cat = Catalog::build();
        let e = &cat.entries[cat.entry_by_text("è").unwrap() as usize];
        assert_eq!(e.name, "e with grave");
        assert_eq!(e.upper, "È");
        assert_eq!(e.upper_name, "E with grave");
        let s = &cat.entries[cat.entry_by_text("ș").unwrap() as usize];
        assert_eq!(s.upper, "Ș");
        let ss = &cat.entries[cat.entry_by_text("ß").unwrap() as usize];
        assert_eq!(ss.upper, "ẞ");
        let i = &cat.entries[cat.entry_by_text("i").unwrap() as usize];
        assert_eq!(i.upper, "İ");
        let viet = &cat.entries[cat.entry_by_text("ấ").unwrap() as usize];
        assert_eq!(viet.name, "a with circumflex and acute");
        let euro = &cat.entries[cat.entry_by_text("€").unwrap() as usize];
        assert!(!euro.is_cased());
    }

    #[test]
    fn search_letters_and_languages() {
        let cat = Catalog::build();
        let order = cat.resolve_order(&["*".to_string()]);
        let v = cat.search("e grave", &order);
        assert_eq!(cat.entries[v[0].entries[0] as usize].lower, "è");
        let v = cat.search("rom", &order);
        assert!(v.iter().any(|s| s.title == "Română"));
        let v = cat.search("română", &order);
        assert!(v.iter().any(|s| s.title == "Română"));
        let v = cat.search("a", &order);
        assert!(v[0].entries.iter().all(|&id| cat.entries[id as usize].tokens.contains(&"a".to_string())));
        // Italian chars rank first for a shared letter.
        assert_eq!(cat.entries[v[0].entries[0] as usize].lower, "à");
        let v = cat.search("euro", &order);
        assert_eq!(cat.entries[v[0].entries[0] as usize].lower, "€");
        let v = cat.search("it", &order);
        assert!(v.iter().any(|s| s.title == "Italiano"));
    }

    #[test]
    fn order_spec() {
        let cat = Catalog::build();
        let order = cat.resolve_order(&["ro".into(), "it".into()]);
        assert_eq!(order.len(), 2);
        let order = cat.resolve_order(&["ro".into(), "*".into()]);
        assert_eq!(cat.sections[order[0]].def.id, "ro");
        assert_eq!(order.len(), cat.sections.len());
    }
}
