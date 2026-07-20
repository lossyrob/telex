[CmdletBinding()]
param(
    [string] $Aumid = 'com.lossyrob.telex.operatorstationspike',

    [string] $SourceHead,

    [string] $OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$python = (Get-Command python -ErrorAction Stop).Source
$database = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\Notifications\wpndatabase.db'
if (-not (Test-Path -LiteralPath $database -PathType Leaf)) {
    throw 'The Windows Action Center database was not found.'
}

$pythonSource = @'
import datetime
import json
import sqlite3
import sys
import xml.etree.ElementTree as ET

database, aumid, source_head = sys.argv[1], sys.argv[2], sys.argv[3]
conn = sqlite3.connect("file:" + database.replace("\\", "/") + "?mode=ro", uri=True)
handler = conn.execute(
    """
    SELECT RecordId, PrimaryId, HandlerType, CreatedTime
    FROM NotificationHandler
    WHERE PrimaryId = ?
    ORDER BY RecordId DESC
    LIMIT 1
    """,
    (aumid,),
).fetchone()
if handler is None:
    raise SystemExit("no Action Center handler found for the Station AUMID")

row = conn.execute(
    """
    SELECT [Order], Id, HandlerId, Type, Payload, ArrivalTime, PayloadType
    FROM Notification
    WHERE HandlerId = ?
    ORDER BY [Order] DESC
    LIMIT 1
    """,
    (handler[0],),
).fetchone()
if row is None:
    raise SystemExit("no Action Center notification found for the Station AUMID")

payload = row[4]
if isinstance(payload, bytes):
    payload = payload.decode("utf-8")
root = ET.fromstring(payload)
texts = root.findall(".//text")
title = texts[0].text if len(texts) > 0 else None
body = texts[1].text if len(texts) > 1 else None
attribution = texts[2].text if len(texts) > 2 else None
arrival = (
    datetime.datetime(1601, 1, 1, tzinfo=datetime.timezone.utc)
    + datetime.timedelta(microseconds=row[5] / 10)
).isoformat().replace("+00:00", "Z")

result = {
    "schema": "operator-station-spike.windows-action-center-evidence.v1",
    "capturedAt": datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z"),
    "source": r"%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db",
    "queryMode": "read-only",
    "extractionScript": "spike/operator-station/harness/Get-OperatorSpikeToastRecord.ps1",
    "sourceHead": source_head or None,
    "handler": {
        "recordId": handler[0],
        "primaryId": handler[1],
        "handlerType": handler[2],
        "createdTime": handler[3],
    },
    "notification": {
        "order": row[0],
        "id": row[1],
        "handlerId": row[2],
        "type": row[3],
        "payloadType": row[6],
        "arrivalTime": arrival,
        "title": title,
        "body": body,
        "bodyCharacterCount": len(body) if body is not None else None,
        "bodyEndsWithEllipsis": body.endswith("…") if body is not None else None,
        "attribution": attribution,
        "payload": payload,
    },
}
print(json.dumps(result, indent=2))
'@

$json = $pythonSource | & $python - $database $Aumid $SourceHead
if ($LASTEXITCODE -ne 0) {
    throw 'Reading the Windows Action Center record failed.'
}
$json | ConvertFrom-Json | Out-Null

if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $parent = Split-Path -Parent $OutputPath
    if ($parent -and -not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    [System.IO.File]::WriteAllText(
        [System.IO.Path]::GetFullPath($OutputPath),
        ($json -join [Environment]::NewLine) + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
}
else {
    $json
}
