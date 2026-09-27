---
title: Format des messages de commit
checks:
  - commit_message: '^\[[a-z0-9_-]+\] \S'
---
Commits atomiques : un commit, une intention. La première ligne commence par ton
rôle entre crochets, suivi d'un espace et d'un résumé à l'impératif, en moins de
72 caractères :

```
[backend] ajoute le cache de sessions
[tests] couvre la liste vide
```

Le corps, s'il y en a un, dit pourquoi — le diff dit déjà quoi. Pas de « wip »,
pas de « fix », pas de « divers » : un message qui ne dit rien sera à réécrire.
Les commits de fusion ne sont pas concernés.
