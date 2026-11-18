# `entity_query`: every function (horizon-client/src/game/native_entity_query.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `entity_query::<Candidate as Clone>::clone` | 168 | &self | derived (not counted) | lifetime-carrying type `Candidate`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entity_query::<NativeEntityInfo as Clone>::clone` | 23 | &self | derived (not counted) | - |
| `entity_query::<NativeEntityInfo as Debug>::fmt` | 23 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entity_query::<NativeEntityInfo as PartialEq>::eq` | 23 | &self | derived (not counted) | - |
| `entity_query::<QueryBudgets as Default>::default` | 66 | - | derived (not counted) | field `0` of `NativeWorldEpoch` is private to another module [return value] |
| `entity_query::<QueryUsage as Clone>::clone` | 58 | &self | derived (not counted) | - |
| `entity_query::<QueryUsage as Debug>::fmt` | 58 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entity_query::<QueryUsage as Default>::default` | 58 | - | derived (not counted) | - |
| `entity_query::<QueryUsage as PartialEq>::eq` | 58 | &self | derived (not counted) | - |
| `entity_query::<Sphere as Clone>::clone` | 129 | &self | derived (not counted) | - |
| `entity_query::Candidate::guest` | 217 | - | excluded | lifetime-carrying type `Candidate`; std type `core::net::socket_addr::SocketAddr` has no codec yet [&argument] |
| `entity_query::Candidate::matches` | 231 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entity_query::Candidate::named` | 181 | - | excluded | lifetime-carrying type `Candidate` |
| `entity_query::Candidate::point` | 200 | - | excluded | lifetime-carrying type `Candidate` |
| `entity_query::Candidate::shared` | 204 | - | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; lifetime-carrying type `Candidate` |
| `entity_query::Candidate::snapshot` | 245 | self | handle-required | lifetime-carrying type `Candidate` [by-value self] |
| `entity_query::Game::native_entities_in` | 529 | &mut self | bound (b, mechanical) | - |
| `entity_query::NativeEntityInfo::new` | 37 | - | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation |
| `entity_query::QueryBudgets::remaining` | 85 | &self | handle-required | field `0` of `NativeWorldEpoch` is private to another module |
| `entity_query::QueryBudgets::reserve` | 100 | &mut self | handle-required | field `0` of `NativeWorldEpoch` is private to another module; returns `&mut`: needs a child handle that locks its parent |
| `entity_query::QueryBudgets::usage` | 73 | &self | handle-required | field `0` of `NativeWorldEpoch` is private to another module |
| `entity_query::QuerySources::candidate_count` | 298 | &self | excluded | impl-level type parameter `Bolts` requires a listed instantiation; impl-level type parameter `Spawners` requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entity_query::QuerySources::visit` | 321 | self | excluded | `dyn Trait` values cannot cross the data ABI; impl-level type parameter `Bolts` requires a listed instantiation; impl-level type parameter `Spawners` requires a listed instantiation; lifetime-carrying type `QuerySources` [by-value self] |
| `entity_query::Sphere::intersects` | 146 | self | bound (b, mechanical) | - |
| `entity_query::Sphere::new` | 136 | - | bound (b, mechanical) | - |
| `entity_query::coordinates` | 262 | - | bound (b, mechanical) | - |
| `entity_query::midpoint` | 266 | - | bound (a) | - |
| `entity_query::player_id` | 258 | - | bound (b, mechanical) | - |
| `entity_query::query_entities` | 472 | - | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built; field `0` of `NativeWorldEpoch` is private to another module; field `0` of `NativeWorldEpoch` is private to another module [&mut argument] |
| `entity_query::require_server_context` | 457 | - | bound (b, mechanical) | - |

## Fields

| field | status |
|---|---|
| `entity_query::NativeEntityInfo.id` | get + set |
| `entity_query::NativeEntityInfo.dimension` | get + set |
| `entity_query::NativeEntityInfo.kind` | get + set |
| `entity_query::NativeEntityInfo.position` | get + set |
| `entity_query::NativeEntityInfo.bounding_box` | get + set |
| `entity_query::NativeEntityInfo.alive` | get + set |
| `entity_query::NativeEntityInfo.player_uuid` | get + set |
| `entity_query::QueryUsage.mod_index` | get + set |
| `entity_query::QueryUsage.queries` | get + set |
| `entity_query::QueryUsage.candidates` | get + set |
| `entity_query::QueryUsage.results` | get + set |
| `entity_query::QueryBudgets.tick` | no accessor: field `0` of `NativeWorldEpoch` is private to another module |
| `entity_query::QueryBudgets.mods` | get + set |
| `entity_query::Sphere.center` | get + set |
| `entity_query::Sphere.radius` | get + set |
| `entity_query::Candidate.id` | no accessor: `Candidate` carries a lifetime (cannot be a handle type) |
| `entity_query::Candidate.dimension` | no accessor: `Candidate` carries a lifetime (cannot be a handle type) |
| `entity_query::Candidate.kind` | no accessor: `Candidate` carries a lifetime (cannot be a handle type) |
| `entity_query::Candidate.position` | no accessor: `Candidate` carries a lifetime (cannot be a handle type) |
| `entity_query::Candidate.bounding_box` | no accessor: `Candidate` carries a lifetime (cannot be a handle type) |
| `entity_query::Candidate.alive` | no accessor: `Candidate` carries a lifetime (cannot be a handle type) |
| `entity_query::Candidate.removed` | no accessor: `Candidate` carries a lifetime (cannot be a handle type) |
| `entity_query::Candidate.player_uuid` | no accessor: `Candidate` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.dimension` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.shared` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.guests` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.others` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.cushions` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.frames` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.paintings` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.potions` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.include_transients` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.thrown_count` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.end` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.eye_count` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.bolts` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.dragon` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.crystals` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.fireballs` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.clouds` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.knots` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.spawners` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
| `entity_query::QuerySources.fangs` | no accessor: `QuerySources` carries a lifetime (cannot be a handle type) |
