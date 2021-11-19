# Get page information
curl "https://api.notion.com/v1/pages/$PAGE_ID" \
  -H "Notion-Version: 2021-08-16" \
  -H "Authorization: Bearer $BEARER_TOKEN"

# Get page as block
curl "https://api.notion.com/v1/blocks/$PAGE_ID" \
  -H "Notion-Version: 2021-08-16" \
  -H "Authorization: Bearer $BEARER_TOKEN"

# Get first 100 children of page
curl "https://api.notion.com/v1/blocks/$PAGE_ID/children?page_size=100" \
  -H "Notion-Version: 2021-08-16" \
  -H "Authorization: Bearer $BEARER_TOKEN"
