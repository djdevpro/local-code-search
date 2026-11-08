# LLM de départ

Critères : ≤ 1,5 milliard de paramètres (exécutable sur CPU), licence permissive, export ONNX
possible, bon niveau en code.

| Candidat | Taille | Licence | Remarque |
|---|---|---|---|
| `Qwen/Qwen2.5-Coder-0.5B-Instruct` | 0,5 B | Apache-2.0 | spécialisé code, très léger |
| `Qwen/Qwen2.5-Coder-1.5B-Instruct` | 1,5 B | Apache-2.0 | meilleure qualité, plus lent |

À vérifier avant de choisir : licence exacte de la version retenue, disponibilité d'un export ONNX
(`onnx-community/…`), latence par requête sur CPU (cible : < 100 ms pour une réécriture courte).
