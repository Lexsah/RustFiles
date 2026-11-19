//! Visible strings. The language follows the operating system UI language.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Strings {
    pub home: &'static str,
    pub favorites: &'static str,
    pub drives: &'static str,
    pub cloud: &'static str,
    pub network: &'static str,
    pub wsl: &'static str,
    pub tags: &'static str,
    pub recycle: &'static str,
    pub settings: &'static str,
    pub desktop: &'static str,
    pub downloads: &'static str,
    pub documents: &'static str,
    pub pictures: &'static str,
    pub music: &'static str,
    pub videos: &'static str,
    pub pin: &'static str,
    pub recent: &'static str,
    pub new: &'static str,
    pub folder: &'static str,
    pub text_file: &'static str,
    pub new_folder_name: &'static str,
    pub new_file_name: &'static str,
    pub search: &'static str,
    pub share_unavailable: &'static str,
    pub network_unavailable: &'static str,
    pub tags_unavailable: &'static str,
    pub cloud_unavailable: &'static str,
    pub wsl_unavailable: &'static str,
    pub empty: &'static str,
    pub no_matches: &'static str,
    pub no_recent: &'static str,
    pub items: &'static str,
    pub selected: &'static str,
    pub details_title: &'static str,
    pub select_one: &'static str,
    pub multiple: &'static str,
    pub name: &'static str,
    pub modified: &'static str,
    pub kind: &'static str,
    pub size: &'static str,
    pub path: &'static str,
    pub folder_kind: &'static str,
    pub light: &'static str,
    pub dark: &'static str,
    pub show_hidden: &'static str,
    pub hide_hidden: &'static str,
    pub reindex: &'static str,
    pub up: &'static str,
    pub files: &'static str,
    pub icons: &'static str,
    pub details: &'static str,
    pub sort: &'static str,
    pub preview: &'static str,
    pub analyze: &'static str,
    pub sunburst: &'static str,
    pub treemap: &'static str,
}

pub fn system_lang() -> Lang {
    #[cfg(windows)]
    {
        let lang = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        if lang & 0x3ff == 0x0c {
            Lang::Fr
        } else {
            Lang::En
        }
    }
    #[cfg(not(windows))]
    {
        let lang = std::env::var("LANG")
            .or_else(|_| std::env::var("LC_MESSAGES"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if lang.starts_with("fr") {
            Lang::Fr
        } else {
            Lang::En
        }
    }
}

pub fn strings(lang: Lang) -> Strings {
    match lang {
        Lang::En => EN,
        Lang::Fr => FR,
    }
}

const EN: Strings = Strings {
    home: "Home",
    favorites: "Favorites",
    drives: "Drives",
    cloud: "Cloud drives",
    network: "Network",
    wsl: "WSL",
    tags: "Tags",
    recycle: "Recycle bin",
    settings: "Settings",
    desktop: "Desktop",
    downloads: "Downloads",
    documents: "Documents",
    pictures: "Pictures",
    music: "Music",
    videos: "Videos",
    pin: "Pin current folder",
    recent: "Recent",
    new: "New",
    folder: "Folder",
    text_file: "Text document",
    new_folder_name: "New folder",
    new_file_name: "New Text Document.txt",
    search: "Search",
    share_unavailable: "Sharing is not available in this version.",
    network_unavailable: "Network locations are not available in this version.",
    tags_unavailable: "Tags are not available in this version.",
    cloud_unavailable: "Cloud drives are not available in this version.",
    wsl_unavailable: "WSL locations are not available in this version.",
    empty: "This folder is empty",
    no_matches: "No matching files",
    no_recent: "No recent files in Desktop, Documents, or Downloads.",
    items: "items",
    selected: "selected",
    details_title: "Details",
    select_one: "Select a file to see its details.",
    multiple: "Multiple items selected.",
    name: "Name",
    modified: "Date modified",
    kind: "Type",
    size: "Size",
    path: "Path",
    folder_kind: "Folder",
    light: "Light",
    dark: "Dark",
    show_hidden: "Show hidden",
    hide_hidden: "Hide hidden",
    reindex: "Reindex",
    up: "Up",
    files: "Files",
    icons: "Icons",
    details: "Details",
    sort: "Sort",
    preview: "Preview",
    analyze: "Analyze",
    sunburst: "Sunburst",
    treemap: "Treemap",
};

const FR: Strings = Strings {
    home: "Accueil",
    favorites: "Favoris",
    drives: "Lecteurs",
    cloud: "Lecteurs cloud",
    network: "Réseau",
    wsl: "WSL",
    tags: "Étiquettes",
    recycle: "Corbeille",
    settings: "Paramètres",
    desktop: "Bureau",
    downloads: "Téléchargements",
    documents: "Documents",
    pictures: "Images",
    music: "Musique",
    videos: "Vidéos",
    pin: "Épingler ce dossier",
    recent: "Récent",
    new: "Nouveau",
    folder: "Dossier",
    text_file: "Document texte",
    new_folder_name: "Nouveau dossier",
    new_file_name: "Nouveau document texte.txt",
    search: "Rechercher",
    share_unavailable: "Le partage n'est pas disponible dans cette version.",
    network_unavailable: "Les emplacements réseau ne sont pas disponibles dans cette version.",
    tags_unavailable: "Les étiquettes ne sont pas disponibles dans cette version.",
    cloud_unavailable: "Les lecteurs cloud ne sont pas disponibles dans cette version.",
    wsl_unavailable: "Les emplacements WSL ne sont pas disponibles dans cette version.",
    empty: "Ce dossier est vide",
    no_matches: "Aucun fichier correspondant",
    no_recent: "Aucun fichier récent sur le Bureau, dans Documents ou Téléchargements.",
    items: "éléments",
    selected: "sélectionné(s)",
    details_title: "Détails",
    select_one: "Sélectionnez un fichier pour voir ses détails.",
    multiple: "Plusieurs éléments sélectionnés.",
    name: "Nom",
    modified: "Date de modification",
    kind: "Type",
    size: "Taille",
    path: "Chemin",
    folder_kind: "Dossier",
    light: "Clair",
    dark: "Sombre",
    show_hidden: "Afficher les masqués",
    hide_hidden: "Masquer les masqués",
    reindex: "Réindexer",
    up: "Haut",
    files: "Fichiers",
    icons: "Icônes",
    details: "Détails",
    sort: "Trier",
    preview: "Aperçu",
    analyze: "Analyser",
    sunburst: "Anneau",
    treemap: "Carte",
};
