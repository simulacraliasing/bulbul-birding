"""Export the portable birding tool schemas from an explicitly supplied public OpenAPI contract."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DESTINATION = ROOT / "assets/tools.json"
# The explicit list keeps admin, worker and unrelated user APIs out of the harness.
OPERATIONS = [
    ("identify_photo", "POST", "/bird-identification/identify", "上传本地鸟类照片，使用云端 Geomodel 按拍摄地点和日期校准", False, None),
    ("identify_audio", "POST", "/bird-identification/audio/identify", "上传本地鸟声文件，使用云端 BirdNET 与 Geomodel 识别并返回时间片段", False, None),
    ("hotspots_nearby", "GET", "/hotspots/nearby", "查询附近观鸟热点", False, None),
    ("hotspots_species", "GET", "/hotspots/{source}/{hotspot_id}/species", "查询热点鸟种", False, None),
    ("hotspots_species_matches", "POST", "/hotspots/species-matches", "匹配目标鸟种与热点", False, None),
    ("hotspots_target_destinations", "GET", "/hotspots/target-destinations", "按目标 taxon_id 推荐观鸟地点", False, None),
    ("species_predict", "GET", "/species/predict", "预测地点在指定日期的鸟种；与网页一样保存最近查询", False, "items"),
    ("species_catalog", "GET", "/species/catalog", "搜索鸟种目录，解析项目内 taxon_id", False, None),
    ("species_wiki", "GET", "/species/{taxon_id}/wiki", "读取鸟种百科", False, None),
    ("species_detail", "POST", "/species/{taxon_id}/detail", "读取鸟种详情和相似种", False, None),
    ("species_range_map", "GET", "/species/{taxon_id}/range-map", "读取鸟种分布图元数据", False, None),
    ("species_etymology", "GET", "/species/{taxon_id}/etymology", "读取鸟种名称词源", False, None),
    ("quiz_taxonomy_options", "GET", "/quiz/custom-taxonomy-options", "读取可用于练习的分类范围", False, None),
    ("saved_queries", "GET", "/me/saved-queries", "读取当前用户最近的地点预测", False, "snapshots"),
    ("species_lists", "GET", "/me/species-lists", "读取当前用户的鸟种清单摘要", False, "lists"),
    ("species_list_detail", "GET", "/me/species-lists/{list_id}", "分页读取当前用户的鸟种清单", False, None),
    ("quiz_route_book_create", "POST", "/quiz/route-book", "生成路线书练习；取得用户授权后传 confirm=true", True, None),
    ("quiz_custom_batch_create", "POST", "/quiz/custom-batch", "生成自定义练习；取得用户授权后传 confirm=true", True, None),
    ("species_list_create", "POST", "/me/species-lists", "保存鸟种清单；取得用户授权后传 confirm=true", True, None),
    ("species_list_from_saved_query", "POST", "/me/species-lists/from-saved-query", "从最近查询保存清单；取得用户授权后传 confirm=true", True, None),
    ("species_list_from_taxonomy", "POST", "/me/species-lists/from-taxonomy", "从分类范围保存清单；取得用户授权后传 confirm=true", True, None),
    ("species_list_replace_items", "PUT", "/me/species-lists/{list_id}/items", "替换清单全部鸟种；取得用户授权后传 confirm=true", True, None),
]


def build_tools(contract: dict) -> list[dict]:
    def resolve(value):
        if isinstance(value, list):
            return [resolve(item) for item in value]
        if not isinstance(value, dict):
            return value
        if "$ref" in value:
            if not value["$ref"].startswith("#/"):
                raise ValueError("Only local OpenAPI references are supported")
            target = contract
            for part in value["$ref"].removeprefix("#/").split("/"):
                target = target[part.replace("~1", "/").replace("~0", "~")]
            return {**resolve(target), **resolve({key: item for key, item in value.items() if key != "$ref"})}
        return {key: resolve(item) for key, item in value.items()}

    tools = []
    for name, method, path, description, mutation, result_limit in OPERATIONS:
        operation = contract["paths"][f"/api/v1{path}"][method.lower()]
        properties, required, path_keys, query_keys, body_keys = {}, [], [], [], []
        for parameter in operation.get("parameters", []):
            if parameter["in"] not in {"path", "query"}:
                continue
            key = parameter["name"]
            properties[key] = resolve(parameter["schema"])
            if parameter.get("description"):
                properties[key]["description"] = parameter["description"]
            if parameter.get("required"):
                required.append(key)
            (path_keys if parameter["in"] == "path" else query_keys).append(key)
        upload = None
        if "requestBody" in operation:
            content = operation["requestBody"]["content"]
            if "multipart/form-data" in content:
                body = resolve(content["multipart/form-data"]["schema"])
                file = body["properties"]["file"]
                if file.get("format") != "binary" and file.get("contentMediaType") != "application/octet-stream":
                    raise ValueError("Upload operation needs one binary file field")
                properties["file"] = {"type": "string", "minLength": 1, "description": "需要上传的本地文件绝对路径；声音完整文件最多 5 分钟、50 MB，选区不能绕过时长限制；不支持远程 URL" if name == "identify_audio" else "需要上传的本地文件绝对路径；不支持远程 URL"}
                required.append("file")
                upload = {"file_key": "file", "max_file_bytes": (20 if name == "identify_photo" else 50) * 1024 * 1024,
                          "timeout_seconds": 90 if name == "identify_photo" else 230}
                # Local Geomodel weights remain an application-only form field.
                if name == "identify_photo":
                    properties["cloud_calibration"] = {"type": "boolean", "const": True, "default": True}
            else:
                body = resolve(content["application/json"]["schema"])
                properties.update(body["properties"])
                required.extend(body.get("required", []))
                body_keys.extend(body["properties"])
        if result_limit:
            properties["limit"] = {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
        if mutation:
            properties["confirm"] = {"type": "boolean", "default": False, "description": "用户已授权本次操作时设为 true；false 只返回操作预览"}
        tools.append({
            **(upload or {}),
            "name": name, "description": description, "method": method, "path": path,
            "mutation": mutation, "path_keys": path_keys, "query_keys": query_keys,
            "body_keys": body_keys, "has_body": "requestBody" in operation,
            "result_limit": result_limit,
            "input_schema": {"type": "object", "properties": properties, "required": required, "additionalProperties": False},
        })
    tools.append({
        "name": "location_search", "description": "用天地图搜索地点候选；同名或多候选时请用户选择，再使用返回的坐标",
        "mutation": False,
        "input_schema": {"type": "object", "properties": {
            "query": {"type": "string", "minLength": 1, "maxLength": 200},
            "limit": {"type": "integer", "minimum": 1, "maximum": 10, "default": 5},
        }, "required": ["query"], "additionalProperties": False},
    })
    return tools


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--openapi", type=Path, required=True, help="Public API contract JSON to read")
    parser.add_argument("--output", type=Path, default=DESTINATION, help="Tool schema JSON to write or check")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        contract = json.loads(args.openapi.read_text(encoding="utf-8"))
        expected = json.dumps(build_tools(contract), ensure_ascii=False, indent=2) + "\n"
        if args.check:
            if not args.output.exists() or args.output.read_text(encoding="utf-8") != expected:
                parser.error("Birding tools are stale; run this command without --check")
        else:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(expected, encoding="utf-8", newline="\n")
    except (OSError, ValueError, KeyError) as exc:
        parser.error(f"Cannot export birding tools: {exc}")
    print(f"Birding tools: {len(OPERATIONS) + 1}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
