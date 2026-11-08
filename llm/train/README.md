# Fine-tuning

Méthode prévue : LoRA sur le LLM de `base/`, avec les données de `data/`.

Étapes :
1. Séparer `data/` en entraînement / validation (90 / 10), par dépôt pour éviter les fuites.
2. Entraîner l'adaptateur LoRA (quelques époques, taux d'apprentissage bas).
3. Fusionner l'adaptateur dans les poids, puis passer à `export/`.
4. Mesurer avec le banc d'essai (`lcs bench`) avant toute intégration.

Outils envisagés : `transformers` + `peft` (Python). Rien n'est exécuté par le moteur Rust à cette étape.
