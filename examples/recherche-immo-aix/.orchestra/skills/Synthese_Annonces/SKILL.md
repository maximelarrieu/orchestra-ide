---
name: Synthèse_Annonces
description: Compile des annonces immobilières collectées en un rapport Markdown classé par pertinence. À utiliser une fois les annonces récupérées/filtrées, pour produire la livraison finale.
---

# Synthèse des annonces

> **Modèle de fiche de skill.** Un `SKILL.md` n'est *pas* du code : c'est une fiche
> d'instructions injectée dans le prompt de tout agent à qui ce skill est assigné. La
> structure ci-dessous (Quand · Étapes · Outils · Sortie · Qualité) est l'ossature
> recommandée — duplique-la et adapte chaque section à ton besoin.

## Quand l'utiliser
Déclenche ce skill **uniquement** quand des annonces ont déjà été collectées et filtrées
(fichiers présents dans le workspace) et que l'utilisateur attend une **synthèse finale**
classée. Si aucune donnée n'est disponible, signale-le au lieu d'inventer.

## Étapes
1. **Repère** les fichiers de données du workspace (JSON/Markdown d'annonces).
2. **Lis** chaque source avec `Read_File` ; ignore proprement un fichier illisible.
3. **Normalise** chaque annonce : prix, surface, prix/m², localisation, lien.
4. **Classe** par pertinence selon les critères du persona (budget, surface, secteur).
5. **Rédige** le rapport et **écris-le** avec `Write_File_Validated` dans `rapports/synthese.md`.

## Outils à mobiliser
- `Read_File` — charger les annonces collectées.
- `Write_File_Validated` — écrire le rapport final.
- `Web_Fetch` *(optionnel)* — compléter une fiche depuis l'URL d'une annonce.

## Format de sortie attendu
Un tableau Markdown trié (meilleure pertinence en haut), suivi d'un court résumé :

```markdown
| # | Prix | Surface | Prix/m² | Secteur | Lien |
|---|------|---------|---------|---------|------|
| 1 | …    | …       | …       | …       | …    |

**Synthèse :** 2-3 phrases sur les meilleures pistes et les compromis.
```

## Critères de qualité (garde-fous)
- N'invente jamais une annonce ni un résultat d'outil : ne reporte que des données lues.
- Indique explicitement les champs manquants (`—`) plutôt que de les estimer.
- Reste concis : pas de remplissage, le tableau + le résumé suffisent.
