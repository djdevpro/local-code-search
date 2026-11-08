# Banc d'essai du LLM

Même protocole que le banc d'essai du moteur (`lcs bench`), deux bras :
- **sans LLM** : la question de l'agent telle quelle ;
- **avec LLM** : la question réécrite, puis la même recherche.

Mesures : fichiers retrouvés sur le banc d'essai local, latence ajoutée par la réécriture
(p50, p95). Critère d'intégration : gain significatif (intervalle de confiance par bootstrap qui
exclut zéro) et latence ajoutée < 100 ms au p95 sur CPU.
