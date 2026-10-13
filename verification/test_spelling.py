"""Fast offline checks for preprocessing; real-model evidence is saved separately."""
import sys,tempfile,unittest
from unittest.mock import patch
from pathlib import Path
import numpy as np,soundfile as sf
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'workers'))
from spelling import letters,slow_replay,transcribe,segmented_replay
class FakeModel:
 def __init__(self,answers):self.answers=iter(answers);self.calls=[]
 def generate(self,*args,**kwargs):
  self.calls.append((args,kwargs))
  return type('Result',(),{'text':next(self.answers)})()
class Spelling(unittest.TestCase):
 def test_only_explicit_letter_names(self):
  for text in ['Camp.','CAMP.','A P L E apple.','C AM P.','']:
   self.assertIsNone(letters(text))
  self.assertEqual(letters('A, p, L, E.'),'APLE')
 def test_pitch_preserving_slow_replay(self):
  x=.2*np.sin(2*np.pi*440*np.arange(16000)/16000)
  y=slow_replay(x);self.assertEqual(len(y),int(16000/.78))
  hz=np.fft.rfftfreq(len(y),1/16000)[np.argmax(abs(np.fft.rfft(y)))];self.assertLess(abs(hz-440),4)
  self.assertTrue(np.isfinite(y).all());self.assertLess(np.max(np.abs(y)),.21)
 def test_retry_does_not_know_expected_word(self):
  with tempfile.TemporaryDirectory() as d:
   p=str(Path(d)/'audio.wav');sf.write(p,np.ones(8000)*.1,16000)
   m=FakeModel(['CMP.','C M P.']);r=transcribe(m,p,'spell')
   self.assertEqual(r['text'],'C M P.');self.assertEqual(r['recognition']['method'],'slow_replay')
   self.assertEqual(len(m.calls),2)
   self.assertNotIn('expected',m.calls[0][1]);self.assertNotIn('hotwords',m.calls[0][1])
 def test_whole_words_stay_unresolved(self):
  with tempfile.TemporaryDirectory() as d:
   p=str(Path(d)/'audio.wav');sf.write(p,np.ones(8000)*.1,16000)
   r=transcribe(FakeModel(['Camp.','Camp.']),p,'spell');self.assertIsNone(letters(r['text']))
 def test_reading_context_does_not_contain_the_answer(self):
  m=FakeModel(['Camp.']);r=transcribe(m,'unused','read')
  self.assertEqual(r['text'],'Camp.');self.assertEqual(r['recognition']['method'],'word_reading')
  self.assertIn('word or phrase',m.calls[0][1]['system_prompt'])
  self.assertNotIn('camp',m.calls[0][1]['system_prompt'].lower())
  self.assertNotIn('hotwords',m.calls[0][1])
 def run_partition_case(self, answers):
  with patch('spelling.pause_boundaries', return_value=[0,8000,16000,24000]), patch('spelling.candidate_partitions',return_value=[(0,1,3),(0,2,3)]):
   return segmented_replay(FakeModel(answers),np.ones(24000,dtype='float32')*.1)
 def test_variable_size_groups_need_two_complete_matches(self):
  text,passes=self.run_partition_case(['L A','N D S C A P E','L A N D S','C A P E'])
  self.assertEqual(text,'L A N D S C A P E')
  self.assertEqual(sum(p['method']=='complete_partition'for p in passes),2)
 def test_single_letter_is_only_allowed_inside_a_partition(self):
  self.assertIsNone(letters('P.'))
  self.assertEqual(letters('P.',minimum=1),'P')
  self.assertIsNone(letters('Camp.',minimum=1))
  self.assertIsNone(letters('Please repeat P.',minimum=1))
 def test_actual_camp_failure_outputs_join_single_letter(self):
  # Logged real failure: CAM + P was rejected before CA + MP got only one vote.
  text,_=self.run_partition_case(['C A M.','P.','C A.','M P.'])
  self.assertEqual(text,'C A M P')
 def test_single_letter_wrong_spelling_is_not_completed(self):
  text,_=self.run_partition_case(['C.','A P.','C A.','P.'])
  self.assertEqual(text,'C A P')
 def test_repeated_letters_survive_disjoint_join(self):
  text,_=self.run_partition_case(['A P','P L E','A P P','L E'])
  self.assertEqual(text,'A P P L E')
 def test_partitions_reject_disagreement(self):
  text,_=self.run_partition_case(['L A','N D S C A P E','L A M D S','C A P E'])
  self.assertIsNone(text)
 def test_partitions_do_not_convert_words_to_letters(self):
  text,_=self.run_partition_case(['Land','Landscape'])
  self.assertIsNone(text)
if __name__=='__main__':unittest.main()
