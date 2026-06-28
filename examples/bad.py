# sklint: strict
"""
Описание тестового модуля
"""

from __future__ import annotations
from dataclasses import dataclass
from typing import Final
import os
from sys import platform

PORT: Final[int] = 8080

# комментарий с маленькой буквы.

type BadAlias = list[int | str]
VALUE=1+2

config = build(
    a=1, b=2
)
values = [
    1,
]

if os.name == "nt":
    VALUE = VALUE+1
if platform == "win32":
    VALUE = VALUE+2

used_before = later_value()


def parse_value(raw: str) -> int:
    """Функция ожидать преобразование строки в число с очень длинным описанием которое превышает лимит семьдесят два символа.

    Args:
        raw (str): исходное значение которое специально описано очень длинно и должно быть перенесено на следующую строку

    Returns:
        int: результат.
    """
    interim = int(raw)
    if interim > 0:
        return interim
    return 0


def collect_values(items):
    result = []
    for item in items:
        result.append(item.value)
    return result


def fail_fast() -> None:
    try:
        raise RuntimeError("bad")
    except RuntimeError:
        raise
    finally:
        pass


@dataclass
class BaseConfig:
    """
    Описание базовой конфигурации

    Attributes:
        host (str): имя сервера
    """

    host: str


@dataclass
class AppConfig(BaseConfig):
    """
    Описание конфигурации приложения

    Attributes:
        port (str): порт сервера
    """

    port: int


class DynamicBox:
    def attach(self) -> None:
        self.generated = 1

    def read(self) -> int:
        return self.generated


def later_value() -> int:
    return 1


print("suppressed debug")  # noqa: SK201

box = DynamicBox()
box.attach()
print(box.read())
