//! vectors.bin : journal d'enregistrements de taille fixe (ajout, suppression), vecteurs en
//! f16, CRC par enregistrement, recherche exacte sur fichier mappé en mémoire.
//! Compaction atomique (fichier temporaire + fsync + renommage).

use anyhow::{Context, Result, bail};
use half::f16;
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"LCSVEC01";
const HEADER: usize = 8 + 4 + 32;
const PUT: u8 = 1;
const DEL: u8 = 0;

pub struct Store {
    path: PathBuf,
    dim: usize,
    identity: String,
    file: File,
    /// id → position du vecteur dans le fichier.
    offsets: HashMap<u64, u64>,
    dead: usize,
    /// Vecteurs décodés en f32 (ids, matrice), reconstruits après chaque écriture.
    cache: std::cell::RefCell<Option<(Vec<u64>, Vec<f32>)>>,
}

fn record_len(dim: usize) -> usize {
    1 + 8 + dim * 2 + 4
}

fn crc(bytes: &[u8]) -> [u8; 4] {
    blake3::hash(bytes).as_bytes()[..4].try_into().expect("4 octets")
}

fn header(dim: usize, identity: &str) -> Vec<u8> {
    let mut h = MAGIC.to_vec();
    h.extend((dim as u32).to_le_bytes());
    let mut id = [0u8; 32];
    let bytes = identity.as_bytes();
    id[..bytes.len().min(32)].copy_from_slice(&bytes[..bytes.len().min(32)]);
    h.extend(id);
    h
}

/// Ouvre (ou recrée) le fichier ; renvoie aussi une note si les vecteurs ont été invalidés.
pub fn open(path: &Path, dim: usize, identity: &str) -> Result<(Store, Option<String>)> {
    let expected = header(dim, identity);
    let mut note = None;
    let existing = std::fs::read(path).ok();
    let valid = existing.as_ref().is_some_and(|b| b.len() >= HEADER && b[..HEADER] == expected[..]);
    if existing.is_some() && !valid {
        note = Some("modèle d'embedding changé : les vecteurs sont recalculés".to_string());
    }
    if !valid {
        let mut f = File::create(path).with_context(|| format!("création de {}", path.display()))?;
        f.write_all(&expected)?;
        f.sync_all()?;
    }
    let bytes = if valid { existing.unwrap_or_default() } else { expected.clone() };
    let (offsets, dead, good_len) = scan(&bytes, dim);
    if good_len < bytes.len() {
        note = Some(format!("{} octets corrompus en fin de vectors.bin ignorés", bytes.len() - good_len));
    }
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    file.set_len(good_len as u64)?;
    let store = Store { path: path.to_path_buf(), dim, identity: identity.to_string(), file, offsets, dead, cache: Default::default() };
    Ok((store, note))
}

/// Nombre de vecteurs vivants, sans charger le modèle (pour `status`).
pub fn peek(path: &Path) -> Option<usize> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < HEADER || &bytes[..8] != MAGIC {
        return None;
    }
    let dim = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    Some(scan(&bytes, dim).0.len())
}

fn scan(bytes: &[u8], dim: usize) -> (HashMap<u64, u64>, usize, usize) {
    let len = record_len(dim);
    let mut offsets = HashMap::new();
    let mut dead = 0;
    let mut pos = HEADER;
    while pos + len <= bytes.len() {
        let rec = &bytes[pos..pos + len];
        if crc(&rec[..len - 4]) != rec[len - 4..] {
            break;
        }
        let id = u64::from_le_bytes(rec[1..9].try_into().expect("8 octets"));
        let previous = match rec[0] {
            PUT => offsets.insert(id, (pos + 9) as u64),
            _ => offsets.remove(&id),
        };
        dead += usize::from(previous.is_some()) + usize::from(rec[0] == DEL);
        pos += len;
    }
    (offsets, dead, pos)
}

/// Clé du cache par contenu : empreinte du texte. Le modèle est dans l'en-tête du fichier
/// (`identity`), qui invalide tout le cache quand il change.
pub fn content_key(text: &str) -> u64 {
    u64::from_le_bytes(blake3::hash(text.as_bytes()).as_bytes()[..8].try_into().expect("8 octets"))
}

/// Morceaux vivants répartis entre le magasin (id → vecteur) et le cache (contenu → vecteur).
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Dans le magasin, pas encore dans le cache : à y copier.
    pub seed: Vec<(u64, u64)>,
    /// Absent du magasin, présent dans le cache : repris sans calcul.
    pub reuse: Vec<(u64, u64)>,
    /// Contenu jamais vu : à calculer (id, clé, texte).
    pub compute: Vec<(u64, u64, String)>,
}

pub fn plan(texts: Vec<(u64, String)>, have: &HashSet<u64>, cached: impl Fn(u64) -> bool) -> Plan {
    let mut p = Plan::default();
    for (id, text) in texts {
        let key = content_key(&text);
        match (have.contains(&id), cached(key)) {
            (true, true) => {}
            (true, false) => p.seed.push((id, key)),
            (false, true) => p.reuse.push((id, key)),
            (false, false) => p.compute.push((id, key, text)),
        }
    }
    p
}

impl Store {
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn ids(&self) -> HashSet<u64> {
        self.offsets.keys().copied().collect()
    }

    pub fn contains(&self, id: u64) -> bool {
        self.offsets.contains_key(&id)
    }

    /// Les vecteurs de ces ids, dans le même ordre (None : absent).
    pub fn get_many(&self, ids: &[u64]) -> Result<Vec<Option<Vec<f32>>>> {
        // SAFETY : le fichier n'est jamais tronqué pendant que ce processus le lit.
        let map = unsafe { memmap2::Mmap::map(&self.file)? };
        Ok(ids
            .iter()
            .map(|id| {
                let off = *self.offsets.get(id)? as usize;
                let data = map.get(off..off + self.dim * 2)?;
                Some(data.as_chunks::<2>().0.iter().map(|b| f16::from_le_bytes(*b).to_f32()).collect())
            })
            .collect())
    }

    /// Ramène le fichier à `max` vecteurs en supprimant les plus anciennement écrits, sauf ceux de
    /// `keep`. Renvoie le nombre de vecteurs supprimés.
    pub fn evict_oldest(&mut self, max: usize, keep: &HashSet<u64>) -> Result<usize> {
        if self.offsets.len() <= max {
            return Ok(0);
        }
        let mut old: Vec<(u64, u64)> = self.offsets.iter().filter(|(id, _)| !keep.contains(id)).map(|(&id, &off)| (off, id)).collect();
        old.sort_unstable();
        let n = (self.offsets.len() - max).min(old.len());
        let ids: Vec<u64> = old[..n].iter().map(|&(_, id)| id).collect();
        self.delete(&ids)?;
        Ok(n)
    }

    fn encode(&self, op: u8, id: u64, v: Option<&[f32]>) -> Vec<u8> {
        let mut rec = Vec::with_capacity(record_len(self.dim));
        rec.push(op);
        rec.extend(id.to_le_bytes());
        for i in 0..self.dim {
            let x = v.map_or(0.0, |v| v[i]);
            rec.extend(f16::from_f32(x).to_le_bytes());
        }
        let c = crc(&rec);
        rec.extend(c);
        rec
    }

    fn append(&mut self, records: Vec<(u8, u64, Option<Vec<f32>>)>) -> Result<()> {
        let mut pos = self.file.seek(SeekFrom::End(0))?;
        let mut buf = Vec::new();
        for (op, id, v) in records {
            if v.as_ref().is_some_and(|v| v.len() != self.dim) {
                bail!("dimension inattendue pour le vecteur {id}");
            }
            buf.extend(self.encode(op, id, v.as_deref()));
            let previous = match op {
                PUT => self.offsets.insert(id, pos + 9),
                _ => self.offsets.remove(&id),
            };
            self.dead += usize::from(previous.is_some()) + usize::from(op == DEL);
            pos += record_len(self.dim) as u64;
        }
        self.file.write_all(&buf)?;
        self.file.sync_data()?;
        self.cache.replace(None);
        Ok(())
    }

    pub fn put(&mut self, items: Vec<(u64, Vec<f32>)>) -> Result<()> {
        self.append(items.into_iter().map(|(id, v)| (PUT, id, Some(v))).collect())
    }

    pub fn delete(&mut self, ids: &[u64]) -> Result<()> {
        let present: Vec<u64> = ids.iter().copied().filter(|id| self.offsets.contains_key(id)).collect();
        self.append(present.into_iter().map(|id| (DEL, id, None)).collect())
    }

    /// Recherche exacte : produit scalaire avec chaque vecteur (normalisés → cosinus), sur une
    /// matrice f32 contiguë gardée en mémoire (boucle vectorisée par le compilateur).
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
        if self.offsets.is_empty() || query.len() != self.dim {
            return Ok(Vec::new());
        }
        if self.cache.borrow().is_none() {
            self.cache.replace(Some(self.decode()?));
        }
        let cache = self.cache.borrow();
        let (ids, matrix) = cache.as_ref().expect("cache construit");
        let mut scored: Vec<(u64, f32)> = ids
            .iter()
            .zip(matrix.chunks_exact(self.dim))
            .map(|(&id, v)| (id, v.iter().zip(query).map(|(a, b)| a * b).sum::<f32>()))
            .collect();
        let k = k.min(scored.len());
        if k < scored.len() {
            scored.select_nth_unstable_by(k, |a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            scored.truncate(k);
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(scored)
    }

    fn decode(&self) -> Result<(Vec<u64>, Vec<f32>)> {
        // SAFETY : le fichier n'est jamais tronqué pendant que ce processus le lit.
        let map = unsafe { memmap2::Mmap::map(&self.file)? };
        let mut entries: Vec<(u64, u64)> = self.offsets.iter().map(|(&id, &off)| (id, off)).collect();
        entries.sort_unstable();
        let mut ids = Vec::with_capacity(entries.len());
        let mut matrix = Vec::with_capacity(entries.len() * self.dim);
        for (id, off) in entries {
            let Some(data) = map.get(off as usize..off as usize + self.dim * 2) else { continue };
            ids.push(id);
            matrix.extend(data.as_chunks::<2>().0.iter().map(|b| f16::from_le_bytes(*b).to_f32()));
        }
        Ok((ids, matrix))
    }

    /// Réécrit le fichier sans les enregistrements morts quand ils dominent.
    pub fn compact_if_needed(&mut self) -> Result<bool> {
        if self.dead < 1000 || self.dead < self.offsets.len() {
            return Ok(false);
        }
        let map = unsafe { memmap2::Mmap::map(&self.file)? };
        let tmp = self.path.with_extension("bin.tmp");
        let mut out = File::create(&tmp)?;
        out.write_all(&header(self.dim, &self.identity))?;
        let mut ids: Vec<(u64, u64)> = self.offsets.iter().map(|(&i, &o)| (i, o)).collect();
        ids.sort();
        for (id, off) in ids {
            let mut rec = vec![PUT];
            rec.extend(id.to_le_bytes());
            rec.extend(&map[off as usize..off as usize + self.dim * 2]);
            let c = crc(&rec);
            rec.extend(c);
            out.write_all(&rec)?;
        }
        out.sync_all()?;
        drop(map);
        std::fs::rename(&tmp, &self.path)?;
        let (store, _) = open(&self.path, self.dim, &self.identity)?;
        *self = store;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ajout_suppression_recherche_et_reouverture() {
        let dir = std::env::temp_dir().join(format!("lcs-vec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.bin");
        let (mut s, note) = open(&path, 3, "m1").unwrap();
        assert!(note.is_none());
        s.put(vec![(1, vec![1.0, 0.0, 0.0]), (2, vec![0.0, 1.0, 0.0])]).unwrap();
        s.delete(&[2]).unwrap();
        s.put(vec![(3, vec![0.6, 0.8, 0.0])]).unwrap();
        let top = s.search(&[1.0, 0.0, 0.0], 5).unwrap();
        assert_eq!(top.iter().map(|t| t.0).collect::<Vec<_>>(), [1, 3]);
        drop(s);

        let (s, _) = open(&path, 3, "m1").unwrap();
        assert_eq!(s.ids(), HashSet::from([1, 3]));
        drop(s);
        let (s, note) = open(&path, 3, "m2").unwrap();
        assert!(note.is_some() && s.len() == 0, "autre modèle => vecteurs invalidés");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn lecture_par_id_et_eviction_des_plus_anciens() {
        let dir = std::env::temp_dir().join(format!("lcs-vec-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (mut s, _) = open(&dir.join("vectors-cache.bin"), 2, "m").unwrap();
        s.put(vec![(10, vec![1.0, 0.0]), (20, vec![0.0, 1.0]), (30, vec![0.5, 0.5])]).unwrap();
        let got = s.get_many(&[20, 99, 10]).unwrap();
        assert_eq!(got, vec![Some(vec![0.0, 1.0]), None, Some(vec![1.0, 0.0])]);
        // 10 est le plus ancien mais protégé : on supprime 20 (le suivant), pas 30.
        assert_eq!(s.evict_oldest(2, &HashSet::from([10])).unwrap(), 1);
        assert_eq!(s.ids(), HashSet::from([10, 30]));
        assert_eq!(s.evict_oldest(5, &HashSet::new()).unwrap(), 0, "sous la limite : rien");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn repartition_entre_magasin_et_cache() {
        let (a, b, c, d) = ("fn a() {}", "fn b() {}", "fn c() {}", "fn d() {}");
        let cached = HashSet::from([content_key(a), content_key(c)]);
        let texts = vec![(1, a.to_string()), (2, b.to_string()), (3, c.to_string()), (4, d.to_string())];
        let p = plan(texts, &HashSet::from([1, 2]), |k| cached.contains(&k));
        assert_eq!(p.seed, vec![(2, content_key(b))], "dans le magasin, pas dans le cache : à copier");
        assert_eq!(p.reuse, vec![(3, content_key(c))], "absent du magasin mais déjà vu : repris");
        assert_eq!(p.compute, vec![(4, content_key(d), d.to_string())], "jamais vu : à calculer");
    }
}
