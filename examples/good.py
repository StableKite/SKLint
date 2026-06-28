"""Описание тестового модуля"""
from dataclasses import dataclass
from typing import Final

PORT: Final[int] = 8080
"""Порт сервера"""


# Комментарий без точки


def parse_value(raw: str) -> int:
    """
    Преобразование строки в число

    Args:
        raw (str): исходное значение
    Returns:
        int: результат
    Raises:
        ValueError: если строка не является числом
    """

    return int(raw)



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
        host (str): имя сервера
        port (int): порт сервера
    """

    port: int



class DeclaredBox:
    def __init__(self) -> None:
        self.generated = 1



if __name__ == "__main__":
    declared_box = DeclaredBox()
    print(declared_box.generated)