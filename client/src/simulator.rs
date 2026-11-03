use rand::Rng;
use std::fs::File;
use std::io::{BufRead, BufReader};

pub trait PositionSource {
    fn next_position(&mut self) -> Option<(f64, f64)>;
}

// ---------------------------------------------------------------------------
// Lettura da file CSV
// ---------------------------------------------------------------------------

pub struct FileSource {
    posizioni: Vec<(f64, f64)>,
    indice: usize,
}

impl FileSource {
    fn risolvi_percorso(path: &str) -> std::path::PathBuf {
        let diretto = std::path::Path::new(path);
        if diretto.exists() {
            return diretto.to_path_buf();
        }
        let alternativo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        if alternativo.exists() {
            return alternativo;
        }
        diretto.to_path_buf()
    }

    /// Carica il file e parsa tutte le righe valide in anticipo.
    pub fn new(path: &str) -> anyhow::Result<Self> {
        let percorso = Self::risolvi_percorso(path);
        let file = File::open(&percorso)
            .map_err(|e| anyhow::anyhow!("impossibile aprire '{}': {e}", percorso.display()))?;
        let reader = BufReader::new(file);
        let mut posizioni = Vec::new();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim().to_string();

            // Salta righe vuote e commenti
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parti: Vec<&str> = line.split(',').collect();
            if parti.len() < 2 {
                eprintln!("Riga malformata nel file posizioni (ignorata): {}", line);
                continue;
            }

            // Scarta righe sbagliate
            let lat: f64 = match parti[0].trim().parse() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("Riga con latitudine non numerica (ignorata): {}", line);
                    continue;
                }
            };
            let lon: f64 = match parti[1].trim().parse() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("Riga con longitudine non numerica (ignorata): {}", line);
                    continue;
                }
            };

            if !common::coordinate_valide(lat, lon) {
                eprintln!(
                    "Riga con coordinate fuori range o non finite (ignorata): {}",
                    line
                );
                continue;
            }

            posizioni.push((lat, lon));
        }

        if posizioni.is_empty() {
            anyhow::bail!("Il file di posizioni è vuoto o non contiene righe valide");
        }

        println!(
            "[Simulatore] Caricate {} posizioni dal file '{}'",
            posizioni.len(),
            percorso.display()
        );

        Ok(Self {
            posizioni,
            indice: 0,
        })
    }
}

impl PositionSource for FileSource {
    fn next_position(&mut self) -> Option<(f64, f64)> {
        if self.indice < self.posizioni.len() {
            let pos = self.posizioni[self.indice];
            self.indice += 1;
            Some(pos)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Random Walk
// ---------------------------------------------------------------------------

pub struct RandomWalkSource {
    lat_corrente: f64,
    lon_corrente: f64,
    passo_max_gradi: f64,
}

impl RandomWalkSource {
    pub fn new(center_lat: f64, center_lon: f64, raggio_passo_km: f64) -> Self {
        // 1 grado di latitudine ≈ 111 km
        let passo_max_gradi = raggio_passo_km / 111.0;

        println!(
            "[Simulatore] Random walk dal punto ({:.6}, {:.6}), passo ~{:.3} km",
            center_lat, center_lon, raggio_passo_km
        );

        Self {
            lat_corrente: center_lat,
            lon_corrente: center_lon,
            passo_max_gradi,
        }
    }
}

impl PositionSource for RandomWalkSource {
    fn next_position(&mut self) -> Option<(f64, f64)> {
        let mut rng = rand::rng();

        let delta_lat: f64 = rng.random_range(-self.passo_max_gradi..=self.passo_max_gradi);
        let delta_lon: f64 = rng.random_range(-self.passo_max_gradi..=self.passo_max_gradi);

        self.lat_corrente += delta_lat;
        self.lon_corrente += delta_lon;

        // Clampa le coordinate a valori validi
        self.lat_corrente = self.lat_corrente.clamp(-90.0, 90.0);
        self.lon_corrente = self.lon_corrente.clamp(-180.0, 180.0);

        Some((self.lat_corrente, self.lon_corrente))
    }
}

// ---------------------------------------------------------------------------
// Funzione di utilità per costruire la strategia giusta in base alla scelta
// ---------------------------------------------------------------------------

pub enum SimulationStrategy {
    File(FileSource),
    RandomWalk(RandomWalkSource),
}

impl PositionSource for SimulationStrategy {
    fn next_position(&mut self) -> Option<(f64, f64)> {
        match self {
            SimulationStrategy::File(s) => s.next_position(),
            SimulationStrategy::RandomWalk(s) => s.next_position(),
        }
    }
}
