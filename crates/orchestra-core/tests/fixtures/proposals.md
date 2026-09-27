## Ce que j'ai relu

Le cache de sessions (`store/rows.rs`) et ses tests. Deux points bloquants,
détaillés dans `REVIEW.md`.

## Ce qui revient

C'est la troisième fois qu'une liste d'API arrive sans pagination : je propose
d'en faire une habitude plutôt que de le redire à chaque ticket.

PROPOSITION: convention
titre: Toujours paginer les listes d'API
rôles: backend

Toute route qui renvoie une liste accepte `limit` et `offset`, avec une
limite par défaut de 50 et un maximum de 500. Le total est renvoyé à côté.
FIN PROPOSITION

**PROPOSITION : ADR**
Titre : Les sessions vivent dans SQLite, pas dans Redis

```
## Contexte

Un seul processus, un seul fichier : Redis ajouterait un service à surveiller.

## Décision

Le cache de sessions est une table SQLite, purgée au démarrage.

## Conséquences

Pas de partage entre machines ; acceptable tant qu'il n'y a qu'un daemon.
```

VERDICT: corrections
- backend: `store/rows.rs:88` boucle sans fin quand offset dépasse le total
- tests: aucun test ne couvre la liste vide
