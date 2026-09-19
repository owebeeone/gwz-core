"""Small generated GWZ consumer of the separately owned transport schema."""
import os
import json
from pathlib import Path
from taut.ir.load import schema_from_json
from taut.ir.dsl import F, Msg, Ref, schema

owner = schema_from_json(json.loads(Path(os.environ['GWZ_TRANSPORT_SCHEMA']).read_text()))
SCHEMA = schema(*owner.enums.values(), *owner.messages.values(),
                GwzTransportDelivery=Msg(message=F(1, Ref.Envelope)))
