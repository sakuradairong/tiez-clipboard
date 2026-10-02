Get-StartApps | Where-Object { $_.Name -match 'WeChat|QQ|Word|Excel|PowerPoint|WPS|Telegram|Feishu|DingTalk|Paint|Notepad' } | Select-Object -First 20 | Format-Table -HideTableHeaders
