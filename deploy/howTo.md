# Callifornia: от сборки до запуска и проверок

Пошаговая инструкция для Windows (PowerShell). Дальше **корень репозитория** — каталог, где лежат `client/`, `deploy/`, `services/`.

## Что понадобится

- [Docker Desktop](https://www.docker.com/products/docker-desktop/) (Compose v2: `docker compose`)
- [Trunk](https://trunkrs.dev/) для сборки клиента (Rust/WASM)
- Клон репозитория с диском и временем на первую сборку образа **sfu** (C++/gRPC — может занять долго)

## 1. Файл окружения `deploy/.env`

Создайте `deploy/.env` (рядом с `compose.yaml`). Минимальный пример для локальной машины:

```env
PUBLIC_HOST=127.0.0.1
CONNECTOR_TOKEN_SECRET=change-me-in-production
SUPERVISOR_SFU_INSTANCES=sfu-1|http://sfu:50051|200
```

- **PUBLIC_HOST** — хост или IP, с которого клиент открывает приложение и WebSocket. Для доступа по LAN укажите IP машины (например `192.168.1.45`), не имя сервиса `signaling`.
- **CONNECTOR_TOKEN_SECRET** — один и тот же секрет для connector и signaling.
- **CONNECTOR_CORS_ALLOWED_ORIGINS** — добавьте только если фронт с другого origin (см. раздел «Режим Trunk» ниже). Для nginx на порту 80 обычно не нужен.

Проверка синтаксиса Compose (опционально):

```powershell
Set-Location <корень репозитория>
docker compose -f deploy/compose.yaml --env-file deploy/.env config
```

Если команда завершилась без ошибок — переменные подставились корректно.

## 2. Сборка фронтенда

Статика должна оказаться в `client/dist` — её раздаёт nginx в контейнере `proxy`.

```powershell
Set-Location <корень репозитория>\client
trunk build --release
Set-Location ..
```

Убедитесь, что в `client/dist` появились файлы (в т.ч. `index.html`).

## 3. Запуск стека

Из корня репозитория:

```powershell
Set-Location <корень репозитория>
docker compose -f deploy/compose.yaml --env-file deploy/.env up -d --build
```

- **Первый запуск**: дождитесь healthy у `redis`, затем долгой сборки/старта **sfu** (healthcheck с `start_period` до ~3 минут ожидания порта — это нормально).
- Логи при сбоях:

```powershell
docker compose -f deploy/compose.yaml --env-file deploy/.env logs -f --tail=100
```

## 4. Проверки после запуска

### Состояние контейнеров

```powershell
docker compose -f deploy/compose.yaml --env-file deploy/.env ps
```

Ожидаем сервисы: `redis`, `sfu`, `signaling`, `connector`, `supervisor`, `proxy` — у работающих без ошибок статус `running` (и при необходимости `healthy` у зависимых).

### HTTP: прокси и сервисы

Подставьте вместо `127.0.0.1` ваш **PUBLIC_HOST**, если он другой.

| Что проверяем | URL |
|---------------|-----|
| UI через nginx | `http://127.0.0.1/` |
| Health через прокси | `http://127.0.0.1/health` |
| Connector напрямую | `http://127.0.0.1:8090/health` |
| Signaling напрямую | `http://127.0.0.1:8080/health` |

Пример в PowerShell:

```powershell
Invoke-WebRequest -Uri http://127.0.0.1/ -UseBasicParsing | Select-Object StatusCode
Invoke-WebRequest -Uri http://127.0.0.1/health -UseBasicParsing | Select-Object StatusCode, Content
Invoke-WebRequest -Uri http://127.0.0.1:8090/health -UseBasicParsing | Select-Object StatusCode, Content
Invoke-WebRequest -Uri http://127.0.0.1:8080/health -UseBasicParsing | Select-Object StatusCode, Content
```

### UI в браузере

Откройте **`http://<PUBLIC_HOST>/`** (при `PUBLIC_HOST=127.0.0.1` это **http://127.0.0.1/**).

Если белый экран или 404 — проверьте наличие `client/dist` после `trunk build` и что контейнер `proxy` запущен.

## 5. Режим разработки: Trunk вместо статики в nginx

1. Поднимите только бэкенд (или оставьте полный compose — главное, чтобы connector слушал `8090`).

2. В `deploy/.env` добавьте CORS для origin Trunk:

```env
CONNECTOR_CORS_ALLOWED_ORIGINS=http://127.0.0.1:8081,http://localhost:8081
```

Перезапустите `connector`, если compose уже был запущен (изменились переменные).

3. В другом окне терминала:

```powershell
Set-Location <корень репозитория>\client
$env:CONNECTOR_BASE_URL="http://127.0.0.1:8090"
trunk serve --port 8081 --address 0.0.0.0
```

4. UI: **http://127.0.0.1:8081/**

## 6. Остановка

```powershell
Set-Location <корень репозитория>
docker compose -f deploy/compose.yaml --env-file deploy/.env down
```

---

Краткое описание ролей сервисов и переменных см. в [README.md](README.md) в этой же папке.
