#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# ///
import asyncio
import os
import subprocess
import sys

SOCKET_PATH = "/tmp/agent.sock"
AGENT_CMD = ["/usr/local/bin/uv", "run", "--directory", "/opt", "python3", "agent.py"]


async def connect_to_agent() -> tuple[asyncio.StreamReader, asyncio.StreamWriter]:
    for attempt in range(60):
        try:
            return await asyncio.open_unix_connection(SOCKET_PATH)
        except (ConnectionRefusedError, FileNotFoundError, OSError):
            pass
        if attempt == 0:
            log_file = open(os.path.expanduser("~/agent.log"), "a")
            subprocess.Popen(
                AGENT_CMD,
                stdout=log_file,
                stderr=log_file,
                env={**os.environ, "PYTHONUNBUFFERED": "1"},
            )
        await asyncio.sleep(0.5)
    raise RuntimeError("agent daemon failed to start after 30s")


async def pipe_stdin_to_socket(stdin_reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    while True:
        data = await stdin_reader.read(4096)
        if not data:
            writer.close()
            return
        writer.write(data)
        await writer.drain()


async def pipe_socket_to_stdout(reader: asyncio.StreamReader) -> None:
    while True:
        data = await reader.read(4096)
        if not data:
            return
        sys.stdout.buffer.write(data)
        sys.stdout.buffer.flush()


async def main() -> None:
    reader, writer = await connect_to_agent()
    loop = asyncio.get_running_loop()

    stdin_reader = asyncio.StreamReader()
    proto = asyncio.StreamReaderProtocol(stdin_reader)
    await loop.connect_read_pipe(lambda: proto, sys.stdin)

    await asyncio.gather(
        pipe_stdin_to_socket(stdin_reader, writer),
        pipe_socket_to_stdout(reader),
        return_exceptions=True,
    )


asyncio.run(main())
