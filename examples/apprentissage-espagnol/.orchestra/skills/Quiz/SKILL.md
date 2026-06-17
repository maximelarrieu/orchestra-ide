---
name: Quiz
description: Génère un quiz de 10 questions à compléter/remplir dans la langue apprise, sur le thème de la leçon en cours. À utiliser quand l'apprenant veut s'exercer ou réviser la leçon courante.
---

# Quiz de langue — 10 questions à compléter

> Fiche d'instructions injectée dans le prompt de l'agent (Tuteur / Correcteur). Le quiz est
> rédigé **dans la langue cible** ; les consignes et le corrigé peuvent expliquer dans la
> langue d'explication du persona.

## Quand l'utiliser
Quand l'apprenant demande un quiz, un exercice ou une révision de la leçon en cours. **Avant**
de générer, identifie dans le persona de l'espace (`.orchestra/persona.md`) :
- la **langue cible** (la langue apprise) et la **langue d'explication** ;
- le **niveau (CECR)** ;
- le **thème de la leçon en cours**.
Au besoin, utilise `Recall` pour retrouver la dernière leçon traitée. Si la langue, le niveau
ou le thème manquent, **pose la question en une phrase** au lieu d'inventer.

## Étapes
1. **Cadre** le quiz : langue cible, niveau, thème de la leçon (grammaire + vocabulaire).
2. **Sélectionne 10 points** à tester sur ce thème, gradués du plus simple au plus difficile
   selon le niveau.
3. **Rédige 10 questions dans la langue cible**, de type « à trous » / à compléter. Varie les
   formes :
   - phrase à compléter avec un blanc `____` (mot de vocabulaire, accord, préposition) ;
   - verbe à conjuguer, donné à l'infinitif entre parenthèses : `(viajar)` ;
   - réplique de dialogue à compléter ;
   - mot manquant à traduire depuis la langue d'explication.
4. **Numérote** de 1 à 10 ; ajoute un court **indice** entre parenthèses quand c'est utile au
   niveau visé (ex. l'infinitif du verbe, ou le mot en langue d'explication).
5. **Prépare un corrigé séparé** : la bonne réponse de chaque item + une explication d'une
   ligne (dans la langue d'explication). Liste les variantes acceptées le cas échéant.
6. **Conserve la trace** : note le thème et la date via `Remember`. Si l'outil
   `Write_File_Validated` est disponible (Agent Documentaliste), enregistre le quiz dans
   `cours/quiz-<thème>.md` (énoncé puis corrigé) ; sinon, restitue-le directement.

## Outils à mobiliser
- `Recall` / `Remember` — retrouver la leçon en cours et y noter le quiz généré.
- `Load_Skill` — (re)charger cette fiche complète au besoin.
- `Write_File_Validated` *(si disponible, ex. Documentaliste)* — sauvegarder le quiz dans les notes.

## Format de sortie attendu
```markdown
## Quiz — <thème> (<langue cible>, niveau <CECR>)

1. <phrase avec ____>  (indice)
2. <verbe à conjuguer> ____  (infinitif)
3. …
… jusqu'à 10

### Corrigé
1. <réponse> — <explication courte>
2. …
```
Exactement **10 questions**, énoncé **dans la langue cible**, corrigé en section distincte.

## Critères de qualité (garde-fous)
- **Exactement 10 questions**, toutes sur le thème de la leçon en cours et calibrées au niveau.
- Langue cible irréprochable : orthographe, **accents** et ponctuation propres à la langue.
- **Un seul blanc clair** par question, **une seule** bonne réponse attendue (ou variantes
  listées dans le corrigé) — pas d'ambiguïté.
- Ne **jamais** donner la réponse dans l'énoncé : le corrigé est une section à part.
- Si langue / niveau / thème sont inconnus, **demande-les** plutôt que d'inventer.
