//! `lcs install` : copie le binaire dans `~/.local/bin` sous le nom `lcs` et ajoute ce dossier au PATH
//! de l'utilisateur (registre sous Windows, profil du shell ailleurs).
//! Se fait aussi sans rien demander : quand le fichier téléchargé est lancé sans commande (double-clic),
//! et à sa première commande tant qu'aucun `lcs` ne répond dans le PATH.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const NAME: &str = if cfg!(windows) { "lcs.exe" } else { "lcs" };

fn home() -> Result<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .context("dossier personnel introuvable")
}

/// Les fichiers publiés s'appellent `lcs-<système>` ; un `lcs` tout court est déjà installé, ou
/// compilé sur place.
fn is_download(exe: &Path) -> bool {
    exe.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with("lcs-"))
}

fn in_path(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir))
}

/// Un `lcs` répond déjà dans le PATH de ce terminal.
fn on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(NAME).is_file()))
}

/// Avant toute commande : un binaire téléchargé s'installe seul tant qu'aucun `lcs` ne répond dans le
/// PATH. Écrit sur stderr, stdout reste à la commande (MCP, JSON). Un échec ne bloque pas la commande.
pub fn auto() {
    let mut args = std::env::args_os().skip(1).peekable();
    if args.peek().is_none() || args.any(|a| a == "install") {
        return; // `run` s'en charge et affiche le détail
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if !is_download(&exe) || on_path() {
        return;
    }
    match install(&exe) {
        Ok((lines, true)) => lines.iter().for_each(|l| eprintln!("{l}")),
        Ok(_) => {}
        Err(e) => eprintln!("lcs : installation automatique impossible ({e:#})"),
    }
}

/// `lcs` lancé sans commande doit-il s'installer (fichier téléchargé, ou aucun `lcs` dans le PATH) ?
/// Sinon il affiche l'aide.
pub fn wanted() -> bool {
    std::env::current_exe().is_ok_and(|exe| is_download(&exe)) || !on_path()
}

/// `lcs install`, et `lcs` sans commande (`bare`) : au double-clic, la fenêtre attend avant de se fermer.
pub fn run(bare: bool) -> Result<()> {
    let result = std::env::current_exe().map_err(anyhow::Error::from).and_then(|exe| install(&exe));
    match &result {
        Ok((lines, _)) => lines.iter().for_each(|l| println!("{l}")),
        Err(e) if bare => eprintln!("Erreur : {e:#}"),
        Err(_) => {}
    }
    if bare {
        pause();
        if result.is_err() {
            std::process::exit(1);
        }
    }
    result.map(|_| ())
}

/// Les lignes à afficher, et vrai si quelque chose a changé.
fn install(exe: &Path) -> Result<(Vec<String>, bool)> {
    let home = home()?;
    let dir = home.join(".local").join("bin");
    let (copied, a) = copy(exe, &dir.join(NAME))?;
    let (declared, b) = add_to_path(&dir, &home)?;
    let next = match in_path(&dir) {
        true => "Tape lcs --help pour voir les commandes.",
        false => "Ouvre un nouveau terminal, puis tape lcs --help pour voir les commandes.",
    };
    Ok((vec![copied, declared, next.to_string()], a || b))
}

fn copy(exe: &Path, dest: &Path) -> Result<(String, bool)> {
    if same(exe, dest) {
        return Ok((format!("{} : déjà en place", dest.display()), false));
    }
    let dir = dest.parent().context("destination sans dossier")?;
    std::fs::create_dir_all(dir).with_context(|| format!("création impossible : {}", dir.display()))?;
    let tmp = dest.with_extension(format!("new-{}", std::process::id()));
    let placed = std::fs::copy(exe, &tmp).and_then(|_| executable(&tmp)).and_then(|()| replace(&tmp, dest));
    if placed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    placed.with_context(|| format!("copie impossible vers {}", dest.display()))?;
    Ok((format!("{} : installé", dest.display()), true))
}

/// Le même fichier, ou une copie identique.
fn same(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    if canon(b).is_some() && canon(a) == canon(b) {
        return true;
    }
    let len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
    len(b).is_some() && len(a) == len(b) && matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

#[cfg(unix)]
fn executable(file: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

fn replace(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    if std::fs::rename(tmp, dest).is_ok() {
        return Ok(());
    }
    // Sous Windows, un lcs.exe en cours d'exécution ne se remplace pas, mais il se renomme.
    let old = dest.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(dest, &old)?;
    std::fs::rename(tmp, dest)
}

#[cfg(windows)]
fn add_to_path(dir: &Path, home: &Path) -> Result<(String, bool)> {
    let dir = dir.to_str().context("chemin du dossier personnel illisible")?;
    let home = home.to_str().context("chemin du dossier personnel illisible")?;
    if !win::add_user_path("Environment", dir, home)? {
        return Ok((format!("{dir} : déjà dans le PATH de l'utilisateur"), false));
    }
    win::announce();
    Ok((format!("{dir} : ajouté au PATH de l'utilisateur"), true))
}

#[cfg(not(windows))]
fn add_to_path(dir: &Path, home: &Path) -> Result<(String, bool)> {
    if in_path(dir) {
        return Ok((format!("{} : déjà dans le PATH", dir.display()), false));
    }
    let shell = std::env::var("SHELL").unwrap_or_default();
    let mut changed = Vec::new();
    for file in profiles(home, &shell, &|p: &Path| p.exists()) {
        if append_line(&file)? {
            changed.push(file.display().to_string());
        }
    }
    Ok(match changed.is_empty() {
        true => (format!("{} : déjà déclaré dans le profil du shell", dir.display()), false),
        false => (format!("{} : ajouté au PATH dans {}", dir.display(), changed.join(", ")), true),
    })
}

/// `list` (PATH Windows, séparé par `;`) avec `dir` à la fin ; `None` s'il y est déjà, écrit en entier
/// ou avec `%USERPROFILE%`.
#[cfg(any(windows, test))]
fn appended(list: &str, dir: &str, home: &str) -> Option<String> {
    let home = home.to_lowercase();
    let norm = |p: &str| {
        let p = p.trim().trim_matches('"').to_lowercase().replace("%userprofile%", &home).replace('/', "\\");
        p.trim_end_matches('\\').to_string()
    };
    let target = norm(dir);
    if list.split(';').any(|p| norm(p) == target) {
        return None;
    }
    let sep = if list.is_empty() || list.ends_with(';') { "" } else { ";" };
    Some(format!("{list}{sep}{dir}"))
}

/// Les fichiers que le shell de l'utilisateur lit à l'ouverture d'un terminal.
#[cfg(any(not(windows), test))]
fn profiles(home: &Path, shell: &str, exists: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let shell = shell.rsplit('/').next().unwrap_or("");
    let has = |name: &str| exists(&home.join(name));
    let mut files = Vec::new();
    if shell == "zsh" || has(".zshrc") || has(".zshenv") {
        files.push(home.join(".zshenv"));
    }
    if shell == "bash" || has(".bashrc") {
        files.push(home.join(".bashrc"));
    }
    // Un shell de connexion bash lit le premier de ces fichiers, et pas .profile s'il en existe un.
    if let Some(login) = [".bash_profile", ".bash_login"].into_iter().find(|n| has(*n)) {
        files.push(home.join(login));
    }
    if has(".profile") || !matches!(shell, "zsh" | "fish") {
        files.push(home.join(".profile"));
    }
    if shell == "fish" || has(".config/fish") {
        files.push(home.join(".config/fish/conf.d/lcs.fish"));
    }
    files
}

/// Ajoute `~/.local/bin` au PATH à la fin du fichier ; faux s'il en parle déjà.
#[cfg(any(not(windows), test))]
fn append_line(file: &Path) -> Result<bool> {
    use std::io::Write;
    let current = std::fs::read_to_string(file).unwrap_or_default();
    if current.contains(".local/bin") {
        return Ok(false);
    }
    let line = match file.extension().is_some_and(|e| e == "fish") {
        true => "set -gx PATH \"$HOME/.local/bin\" $PATH",
        false => "export PATH=\"$HOME/.local/bin:$PATH\"",
    };
    let gap = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("création impossible : {}", dir.display()))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .with_context(|| format!("écriture impossible : {}", file.display()))?;
    write!(f, "{gap}# lcs\n{line}\n").with_context(|| format!("écriture impossible : {}", file.display()))?;
    Ok(true)
}

#[cfg(windows)]
fn pause() {
    if win::own_console() {
        println!("Appuie sur Entrée pour fermer cette fenêtre.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

#[cfg(not(windows))]
fn pause() {}

/// Registre et console de Windows, par les fonctions du système (aucune dépendance en plus).
#[cfg(windows)]
mod win {
    use anyhow::{Result, bail};
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    type Hkey = isize;
    const HKEY_CURRENT_USER: Hkey = 0x8000_0001_u32 as i32 as isize;
    const KEY_QUERY_VALUE: u32 = 0x1;
    const KEY_SET_VALUE: u32 = 0x2;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegCreateKeyExW(
            key: Hkey,
            sub: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            access: u32,
            security: *const c_void,
            out: *mut Hkey,
            disposition: *mut u32,
        ) -> i32;
        fn RegQueryValueExW(key: Hkey, name: *const u16, reserved: *const u32, kind: *mut u32, data: *mut u8, len: *mut u32) -> i32;
        fn RegSetValueExW(key: Hkey, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> i32;
        fn RegCloseKey(key: Hkey) -> i32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageTimeoutW(window: isize, message: u32, wparam: usize, lparam: isize, flags: u32, timeout: u32, result: *mut usize) -> isize;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
    }

    struct Key(Hkey);

    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// Ouvre `HKCU\{sub}`, créée si elle manque.
    fn open(sub: &str) -> Result<Key> {
        let sub = wide(sub);
        let mut key: Hkey = 0;
        let access = KEY_QUERY_VALUE | KEY_SET_VALUE;
        let code = unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, null(), 0, access, null(), &mut key, null_mut()) };
        if code != 0 {
            bail!("ouverture du registre impossible (code {code})");
        }
        Ok(Key(key))
    }

    /// Le texte d'une valeur et son type ; `None` si elle n'existe pas.
    fn read(key: &Key, name: &[u16]) -> Result<Option<(String, u32)>> {
        let (mut kind, mut len) = (0u32, 0u32);
        let code = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), &mut kind, null_mut(), &mut len) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if code != 0 {
            bail!("lecture du registre impossible (code {code})");
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            bail!("le PATH du registre n'est pas du texte (type {kind}), rien n'a changé");
        }
        // Taille en octets, texte en UTF-16 ; une unité de marge pour le zéro final.
        let mut data = vec![0u16; (len as usize).div_ceil(2) + 1];
        let mut len = (data.len() * 2) as u32;
        let code = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), &mut kind, data.as_mut_ptr().cast(), &mut len) };
        if code != 0 {
            bail!("lecture du registre impossible (code {code})");
        }
        data.truncate(len as usize / 2);
        while data.last() == Some(&0) {
            data.pop();
        }
        Ok(Some((String::from_utf16(&data)?, kind)))
    }

    fn write(key: &Key, name: &[u16], value: &str, kind: u32) -> Result<()> {
        let data = wide(value);
        let code = unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, kind, data.as_ptr().cast(), (data.len() * 2) as u32) };
        if code != 0 {
            bail!("écriture du registre impossible (code {code})");
        }
        Ok(())
    }

    /// Ajoute `dir` à la fin de la valeur `Path` de `HKCU\{sub}`, sans toucher au reste ni au type
    /// de la valeur ; faux s'il y était déjà. Au moindre doute à la lecture, rien n'est écrit.
    pub fn add_user_path(sub: &str, dir: &str, home: &str) -> Result<bool> {
        let key = open(sub)?;
        let name = wide("Path");
        let (old, kind) = read(&key, &name)?.unwrap_or((String::new(), REG_EXPAND_SZ));
        let Some(new) = super::appended(&old, dir, home) else { return Ok(false) };
        write(&key, &name, &new, kind)?;
        Ok(true)
    }

    /// Prévient l'Explorateur, pour que les terminaux ouverts ensuite voient le nouveau PATH.
    pub fn announce() {
        let what = wide("Environment");
        let mut result = 0usize;
        unsafe {
            SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, what.as_ptr() as isize, SMTO_ABORTIFHUNG, 5000, &mut result);
        }
    }

    /// La console a été ouverte pour ce seul programme (double-clic), elle se fermera avec lui.
    pub fn own_console() -> bool {
        let mut list = [0u32; 2];
        unsafe { GetConsoleProcessList(list.as_mut_ptr(), 2) == 1 }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn RegDeleteKeyW(key: Hkey, sub: *const u16) -> i32;
        }

        #[test]
        fn path_du_registre_complete_sans_perdre_son_contenu() {
            let sub = format!("Software\\lcs-test-{}", std::process::id());
            let (dir, home, name) = (r"C:\Users\ana\.local\bin", r"C:\Users\ana", wide("Path"));
            assert!(add_user_path(&sub, dir, home).unwrap(), "valeur absente : créée");
            let key = open(&sub).unwrap();
            assert_eq!(read(&key, &name).unwrap(), Some((dir.to_string(), REG_EXPAND_SZ)));

            let old = r"%SystemRoot%\system32;C:\Outils été";
            write(&key, &name, old, REG_SZ).unwrap();
            assert!(add_user_path(&sub, dir, home).unwrap());
            assert!(!add_user_path(&sub, dir, home).unwrap(), "déjà présent");
            assert_eq!(read(&key, &name).unwrap(), Some((format!("{old};{dir}"), REG_SZ)));

            drop(key);
            let sub = wide(&sub);
            assert_eq!(unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, sub.as_ptr()) }, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lcs-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn binaire_telecharge_reconnu() {
        for download in ["C:/dl/lcs-windows-x64.exe", "/tmp/lcs-linux-x64", "/tmp/LCS-macos-arm64 (1)"] {
            assert!(is_download(Path::new(download)), "{download}");
        }
        for installed in ["target/debug/lcs", "C:/bin/lcs.exe", "/home/ana/.local/bin/lcs"] {
            assert!(!is_download(Path::new(installed)), "{installed}");
        }
    }

    #[test]
    fn copie_une_fois_puis_seulement_si_le_contenu_change() {
        let dir = temp("copie");
        let (src, dest) = (dir.join("lcs-linux-x64"), dir.join("bin").join(NAME));
        std::fs::write(&src, b"v1").unwrap();
        assert!(copy(&src, &dest).unwrap().1);
        assert!(!copy(&src, &dest).unwrap().1, "copie identique");
        assert!(!copy(&dest, &dest).unwrap().1, "le binaire installé lui-même");
        std::fs::write(&src, b"v2 plus long").unwrap();
        assert!(copy(&src, &dest).unwrap().1);
        assert_eq!(std::fs::read(&dest).unwrap(), b"v2 plus long");
        let left: Vec<_> = std::fs::read_dir(dir.join("bin")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left.len(), 1, "aucun fichier temporaire laissé : {left:?}");
    }

    #[test]
    fn path_windows_complete_une_seule_fois() {
        let (dir, home) = (r"C:\Users\ana\.local\bin", r"C:\Users\ana");
        assert_eq!(appended("", dir, home).as_deref(), Some(dir));
        assert_eq!(appended(r"C:\a;%X%\b", dir, home), Some(format!(r"C:\a;%X%\b;{dir}")));
        assert_eq!(appended(r"C:\a;", dir, home), Some(format!(r"C:\a;{dir}")));
        for present in [r"C:\a;c:\users\ANA\.local\bin\", r"%USERPROFILE%\.local\bin;C:\a", r#""C:/Users/ana/.local/bin""#] {
            assert_eq!(appended(present, dir, home), None, "{present}");
        }
    }

    #[test]
    fn profils_selon_le_shell() {
        let home = Path::new("/home/ana");
        let names = |shell: &str, present: &[&str]| -> Vec<String> {
            profiles(home, shell, &|p: &Path| present.iter().any(|n| home.join(n) == p))
                .iter()
                .map(|p| p.strip_prefix(home).unwrap().to_string_lossy().replace('\\', "/"))
                .collect()
        };
        assert_eq!(names("/bin/zsh", &[]), [".zshenv"]);
        assert_eq!(names("/bin/bash", &[".bash_profile"]), [".bashrc", ".bash_profile", ".profile"]);
        assert_eq!(names("/usr/bin/fish", &[".bashrc", ".profile"]), [".bashrc", ".profile", ".config/fish/conf.d/lcs.fish"]);
        assert_eq!(names("", &[]), [".profile"]);
    }

    #[test]
    fn profil_complete_une_seule_fois() {
        let dir = temp("profil");
        let rc = dir.join(".bashrc");
        std::fs::write(&rc, "alias ll='ls -l'").unwrap();
        assert!(append_line(&rc).unwrap());
        assert!(!append_line(&rc).unwrap());
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), "alias ll='ls -l'\n# lcs\nexport PATH=\"$HOME/.local/bin:$PATH\"\n");
        let fish = dir.join("fish").join("conf.d").join("lcs.fish");
        assert!(append_line(&fish).unwrap());
        assert_eq!(std::fs::read_to_string(&fish).unwrap(), "# lcs\nset -gx PATH \"$HOME/.local/bin\" $PATH\n");
    }
}
