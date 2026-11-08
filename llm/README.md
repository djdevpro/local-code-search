# Labo LLM (hors v0)

Le moteur fonctionne sans LLM. Ce labo prépare la suite : affiner un **petit LLM existant** pour
une seule tâche, **réécrire la question de l'agent** en mots-clés et noms de symboles avant la
recherche, par exemple :

```
« où est géré le raccourci clavier ? »  →  hotkey shortcut keybinding register_global_shortcut
```

Règle : le LLM n'est intégré au moteur que si le banc d'essai (`lcs bench`) prouve un gain,
latence comprise.

| Dossier | Rôle |
|---|---|
| `base/` | le LLM de départ : identifiant Hugging Face, licence, taille |
| `data/` | jeu d'entraînement `{"question", "rewrite"}` |
| `train/` | fine-tuning (LoRA) |
| `export/` | export en ONNX pour être exécuté par `ort`, comme l'embedder |
| `bench/` | mesure : mêmes jeux d'essai, avec et sans réécriture |

Contraintes héritées de la spec : exécution par `ort` (pas de GGUF), 100 % local, licence permissive.
