# Export ONNX

Le modèle fusionné est exporté en ONNX pour être exécuté par `ort`, comme l'embedder et le reranker :

```bash
optimum-cli export onnx --model <dossier-du-modele-fusionne> --task text-generation-with-past llm/export/model
```

Puis quantification int8 pour le CPU (et une variante fp32 pour le GPU, cf. la leçon de
`gte-modernbert` : sa variante fp16 renvoie des zéros sur DirectML, à tester systématiquement).
Le dossier exporté s'utilise comme un dossier local dans la configuration, sans téléchargement.
