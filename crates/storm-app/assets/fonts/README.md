# Fonts

Inter 4.1 (https://github.com/rsms/inter, SIL Open Font License, see
`Inter-LICENSE.txt`), subset to keep the binary small:

```
pyftsubset Inter-<Weight>.ttf --output-file=Inter-<Weight>.ttf \
  --unicodes="U+0000-024F,U+0259,U+02B0-02FF,U+0300-036F,U+0370-03FF,U+0400-04FF,U+1E00-1EFF,U+2000-206F,U+20A0-20CF,U+2100-218F,U+2190-21FF,U+2200-22FF,U+2460-24FF,U+25A0-25FF,U+2600-26FF,U+FEFF,U+FFFD" \
  --layout-features="kern,liga,calt,tnum,case,ccmp,locl,mark,mkmk" --no-hinting --desubroutinize
```

for Regular, Medium and SemiBold (the weights the UI uses). Characters outside
the subset fall back to the system fonts.
