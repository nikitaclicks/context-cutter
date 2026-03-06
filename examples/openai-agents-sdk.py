from agents import Agent

agent = Agent(
    name="assistant",
    mcp_servers=[
        {
            "server_label": "context-cutter",
            "command": "npx",
            "args": ["-y", "context-cutter-mcp"],
        }
    ],
)
