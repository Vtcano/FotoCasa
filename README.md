# Fotos iPhone Local

Visor web local para ver desde el navegador las fotos guardadas en:

```text
Z:\Fotos_iphone
```

Las fotos no se copian ni se meten en una base de datos. El servidor Rust solo escanea esa carpeta y sirve los archivos al navegador.

El indice rapido se guarda en:

```text
data/fotos.sqlite
```

Ese archivo contiene rutas, nombres, extension, tipo, tamano, fecha/carpeta detectada y campos preparados para pais/ciudad. Las fotos originales siguen en `Z:\Fotos_iphone`.

Tambien guarda `latitude` y `longitude` cuando la foto tiene GPS. Esas coordenadas se conservan para poder montar un mapa mas adelante.

## Ejecutar

Desde esta carpeta:

```powershell
cargo run
```

Abre en el portatil:

```text
http://localhost:3000
```

## Abrir desde el iPhone en la misma WiFi

1. Busca la IP del PC:

```powershell
ipconfig
```

2. Mira la `Direccion IPv4` del adaptador WiFi.

3. En el iPhone abre:

```text
http://IP-DE-TU-PC:3000
```

Ejemplo:

```text
http://192.168.1.45:3000
```

Si no carga desde el iPhone, permite el puerto `3000` en el Firewall de Windows para redes privadas.

## Cambiar la carpeta de fotos

Por defecto usa:

```text
Z:\Fotos_iphone
```

Para usar otra carpeta en PowerShell:

```powershell
$env:PHOTO_ROOT="D:\MisFotos"
cargo run
```

## Formatos incluidos

```text
jpg, jpeg, png, gif, webp, bmp, avif, heic, heif, mp4, mov, m4v
```

Nota: algunos navegadores no muestran bien `HEIC`. En iPhone/Safari suele ir mejor que en navegadores de Windows. Más adelante se pueden generar miniaturas JPG para esos archivos.

## Filtros

La web carga los filtros disponibles desde el indice:

```text
fecha
pais
ciudad
tipo: fotos/videos
busqueda por nombre o carpeta
```

La fecha se detecta desde EXIF si existe, y si no desde la estructura/nombre cuando hay datos tipo `202401`, `202402`, etc.

El filtro de ciudad se activa solo despues de elegir un pais.

## Pais y ciudad

Para asignar pais y ciudad:

1. Pulsa `Actualizar indice` para leer archivos y coordenadas GPS.
2. Pulsa `Asignar lugares` para convertir coordenadas GPS en pais/ciudad.

Si una foto no tiene GPS, o no se puede resolver su ubicacion, se guarda como:

```text
Unknown
```

La geocodificacion usa OpenStreetMap/Nominatim desde el servidor local. Para no hacer una consulta por cada foto, guarda una cache por coordenada redondeada en:

```text
data/fotos.sqlite
```

Esto permite que muchas fotos del mismo sitio compartan una sola consulta.
