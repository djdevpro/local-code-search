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

impl Store {
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn ids(&self) -> HashSet<u64> {
        self.offsets.keys().copied().collect()
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
}
