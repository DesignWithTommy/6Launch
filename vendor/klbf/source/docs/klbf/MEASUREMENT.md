# A shared way to measure frame rate

Every project reports frame rates from different places and in different ways: an empty sea, a menu, a loading screen, one lucky second. Those numbers cannot be compared. This page proposes one method, open to any project, so that we can all compare on equal footing.

## Where to start

- Play from a new game until you first arrive at the male lead's house (about 3 % progress), at the point where you can make your first save in his bed. Save there, sleeping until the morning, so every run starts at the same time of day: the sun's position changes shadows, reflections and the frame rate. Every run starts from this morning save.

### Keep your morning save

The game has one automatic save plus 15 manual slots. The two menus don't show the same list:

- **Save** shows only the 15 manual slots (the automatic save never appears there).
- **Load** shows the automatic save at the top, then the manual slots: up to 16 entries.

So count slots the way each menu shows them. Make the morning save **in a manual slot** and note its number in the Save menu (for example slot 3); in the Load menu the same save appears one line lower, under the automatic one. The automatic save is overwritten as soon as you play on, a manual slot is not.

All 16 live together in one `saves` folder, inside the data folder you set with `VICTORY_DATA_DIR`, in `<title id>\saves` (the title id is the folder with an 8-character hex name). Copying that folder keeps both the automatic save and your numbered slot:

1. Right after making the morning save, quit the game and copy the `saves` folder somewhere safe, for example `D:\benchmark-morning`.
2. Before each measured run: quit the game, put your copy back in place of the `saves` folder, start the game, then **Load** and pick your slot (one line below its Save-menu number, because the automatic save comes first). Continue would load the automatic save instead.

In PowerShell (put your own title id in place of `<title id>`):

```powershell
Copy-Item -Recurse "$env:VICTORY_DATA_DIR\<title id>\saves" "D:\benchmark-morning"
```

```powershell
Remove-Item -Recurse "$env:VICTORY_DATA_DIR\<title id>\saves"; Copy-Item -Recurse "D:\benchmark-morning" "$env:VICTORY_DATA_DIR\<title id>\saves"
```

The first command runs once, after the morning save; the second before each run. Keep your copy to yourself: save files can carry your console profile's ids.

## What to play: the standard drive

1. Load the morning save in the male lead's house.
2. Walk out to his car and get in. The game then asks you, for the first time, to open the map (the View button on an Xbox controller).
3. In the map, press right on the D-pad to cycle through the story objectives. Three are offered; pick the closest one, with the **dollar ($) icon** (about 0.64 miles away): the errand about collecting money that is owed. Set the marker on it.
4. Drive there following the GPS route exactly (no shortcut, no alternative road), as you normally would.
5. Stop the measurement once you are parked at the marker.

Why this route: it comes very early in the game, so anyone can reach it quickly; loading it is one of the fastest loads (you start inside the house, with no cutscene); and the drive crosses streets, junctions and busy blocks, which is where frame rates really drop. Everyone driving the same route gives numbers that can be compared.

Optional, after the drive: stand still in traffic, look at the sky, walk in a quiet spot, and report those peaks separately.

## What to report

1. **Frame rate seen in the world**: the peak in each situation (sky, quiet area on foot, traffic, busy street on foot, driving fast...).
2. **Stability index**: the share of the session spent in each band (18 and more, 15–18, 12–15, 9–12, 5–9, under 5).
3. **Average performance index**: average, median, 5 % low and 1 % low over the whole session.
4. The machine: GPU and its memory, CPU, RAM, driver version, Windows version, console profile (Series S or X), upscaler, and the exact settings or launcher used.
5. Known issues seen during the run (freezes, flicker, missing objects).

Samples are one-second averages taken while the world is on screen (loading screens and menus excluded). With this project, [`tools/fps-session.py`](../../tools/fps-session.py) computes the three tables from the run's log and prints one line to share. The session length comes from the log's own timestamps, and the line ends with the log's fingerprint: anyone you give the log to can rerun the tool and check that the line matches it.

## Rules

- No cherry-picked scenes. If a number comes from a quiet scene, say so.
- Report the bad numbers too. Freezes count.
- One machine is one machine: say that other cards may behave differently.
