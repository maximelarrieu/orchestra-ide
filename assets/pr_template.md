# La description de la pull request

Avant de finir, écris `PR.md` à la racine de ton worktree. **Ne le commite pas** :
c'est la description de la requête, pas un fichier du projet. C'est ce texte qui
sera lu par la personne qui décide de fusionner, souvent des jours après, souvent
sans avoir suivi le ticket.

Suis ce squelette, dans cet ordre, sans en ajouter. Une section qui n'a rien à
dire se supprime — on n'écrit pas « néant ».

```markdown
## Ce que ça change

Deux à quatre lignes, du point de vue de qui utilise le logiciel : ce qui est
possible maintenant et ne l'était pas, ou ce qui ne casse plus. Pas de nom de
fonction dans cette section.

## Pourquoi

Le besoin derrière le ticket, et la contrainte qui a guidé la solution. Si le
brief était ambigu, dis l'interprétation retenue — c'est la première chose qu'un
relecteur conteste.

## Comment

Ce qui ne se lit pas dans le diff : les décisions prises, ce qui a été écarté et
pourquoi. Une liste courte, avec `fichier.rs:88` quand ça situe le lecteur. Si la
branche touche à quelque chose de risqué, c'est ici.

## Vérifications

Les commandes que tu as réellement lancées, avec leur résultat :

| commande | résultat |
| --- | --- |
| `cargo test --workspace` | 349 tests, 0 échec |
| `cargo clippy -- -D warnings` | propre |

N'écris jamais une vérification que tu n'as pas lancée. Une case cochée à tort
coûte plus cher que pas de tableau du tout.

## À regarder en priorité

Où poser les yeux d'abord, et ce qui reste ouvert : limites connues, dette
assumée, ce qu'aucun test ne couvre. Trois lignes suffisent.
```

Deux règles qui valent pour tout le texte :

- **Factuel.** Pas de « améliore significativement », pas d'adjectif qui ne
  s'appuie sur rien. Ce qui a été fait, et ce qui ne l'a pas été.
- **Court.** Une description qu'on ne lit pas jusqu'au bout ne sert à personne.
  Vise une page d'écran.

Tu n'écris ni le titre, ni le pied de page : Orchestra les ajoute, avec le ticket,
la branche, le verdict de la relecture et le coût. Ne les recopie pas.
