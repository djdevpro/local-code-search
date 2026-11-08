# Données d'entraînement

Format JSONL, une paire par ligne :

```json
{"question": "where is the keyboard shortcut handled?", "rewrite": "hotkey shortcut keybinding register_global_shortcut"}
```

Sources prévues :
- tickets réels déjà traités : question = début du ticket, réécriture = noms des symboles
  modifiés par le correctif ;
- questions maison (hors jeux d'évaluation).

Règle : aucune donnée d'évaluation dans l'entraînement (sinon le banc d'essai ne prouve rien).
