# References: the research this work builds on

Face Recognizer is original work ([what is new here](../README.md#what-is-new-here)). It stands on published
research, and this appendix names that research: the articles behind the reasoning, behind each model the app
runs, and behind the methods it uses. For each one it says what the project takes from it.

How to read the marks:

- **In use**: the app does this now.
- **Basis**: a model of the project's own was made starting from it.
- **Reasoning**: it shapes how the app is designed and how its readings are worded; nothing of it runs.
- **Planned**: the project intends to build on it and has not yet; nothing here has been measured.

Licences of the models and of the data they were trained on are in [NOTICE](../NOTICE), not here.

## What a face shows, and what it doesn't

The reasoning behind reading a face in layers and wording every reading as what someone *looks* like
([Why the expression reading is layered](WHY_LAYERED_EXPRESSION.md)).

- Barrett, L. F., Adolphs, R., Marsella, S., Martinez, A. M., & Pollak, S. D. (2019). Emotional expressions
  reconsidered: Challenges to inferring emotion from human facial movements. *Psychological Science in the
  Public Interest*, 20(1), 1–68.
  **Reasoning.** A facial movement is not a dependable sign of a feeling. The app therefore reports a look
  with its evidence and never states what a person feels.
- Ekman, P., & Friesen, W. V. (1978). *Facial Action Coding System*. Consulting Psychologists Press.
  **Reasoning.** Describing a face by its separate movements. The app's face-shape cues (smile, brow, jaw,
  eyes) are kept as measurements of their own, apart from any label.
- Russell, J. A. (1980). A circumplex model of affect. *Journal of Personality and Social Psychology*, 39(6),
  1161–1178.
  **In use.** The two axes, valence and arousal, on which a reading is placed beside its label.
- Aviezer, H., Trope, Y., & Todorov, A. (2012). Body cues, not facial expressions, discriminate between intense
  positive and negative emotions. *Science*, 338(6111), 1225–1229.
  **Reasoning.** The body can tell apart what the face alone cannot, so the body is a channel of its own.
- Mauss, I. B., Levenson, R. W., McCarter, L., Wilhelm, F. H., & Gross, J. J. (2005). The tie that binds?
  Coherence among emotion experience, behavior, and physiology. *Emotion*, 5(2), 175–190.
- Mauss, I. B., & Robinson, M. D. (2009). Measures of emotion: A review. *Cognition and Emotion*, 23(2), 209–237.
  **Reasoning** (both). What a person reports, what they show and what their body does are only loosely tied.
  Each channel is therefore recorded as a measurement of its own, on one clock, and none is taken for another.
- Buolamwini, J., & Gebru, T. (2018). Gender Shades: Intersectional accuracy disparities in commercial gender
  classification. *Proceedings of Machine Learning Research*, 81, 77–91.
  **Reasoning.** Face analysis is not equally accurate for everyone. The project measures its errors person by
  person instead of assuming they are even.

## Each person against their own normal

- Molenaar, P. C. M. (2004). A manifesto on psychology as idiographic science: Bringing the person back into
  scientific psychology, this time forever. *Measurement*, 2(4), 201–218.
  **Reasoning.** What holds on average across people need not hold for one person. Every signal is read
  against that person's own usual range, never against other people's.
- Kenny, D. A., & La Voie, L. (1984). The social relations model. *Advances in Experimental Social Psychology*,
  18, 141–182.
  **Planned.** Separating what is a person's own style from what a particular companion brings out. Today the
  app only keeps a person's usual range apart for alone and in company.

## Deciding from several weak cues

- Green, D. M., & Swets, J. A. (1966). *Signal Detection Theory and Psychophysics*. Wiley.
- Ratcliff, R. (1978). A theory of memory retrieval. *Psychological Review*, 85(2), 59–108.
  **Reasoning** (both). Evidence gathered over time until there is enough to decide, and the trade between
  missing something and raising a false alarm. This is why a reading is a vote over several pictures with a
  margin, and why an alert waits for a signal to last.
- Rabiner, L. R. (1989). A tutorial on hidden Markov models and selected applications in speech recognition.
  *Proceedings of the IEEE*, 77(2), 257–286.
  **In use**, as an option that is off until measured: a small state model that steadies the expression label
  from one reading to the next.
- Guo, C., Pleiss, G., Sun, Y., & Weinberger, K. Q. (2017). On calibration of modern neural networks.
  *Proceedings of the 34th International Conference on Machine Learning* (PMLR 70), 1321–1330.
  **In use.** Temperature scaling, with which the expression model's overconfidence is measured and corrected
  from posed test sessions.

## The body and its movement

- Wallbott, H. G. (1998). Bodily expression of emotion. *European Journal of Social Psychology*, 28(6), 879–896.
- Dael, N., Mortillaro, M., & Scherer, K. R. (2012). Emotion expression in body action and posture. *Emotion*,
  12(5), 1085–1101.
  **Reasoning** (both). How much a body moves follows arousal well. The app measures movement, head height,
  shoulders and lean as numbers.
- Tracy, J. L., & Robins, R. W. (2004). Show your pride: Evidence for a discrete emotion expression.
  *Psychological Science*, 15(3), 194–197.
  **Reasoning.** Some states have a typical posture.
- Kleinsmith, A., & Bianchi-Berthouze, N. (2013). Affective body expression perception and recognition: A
  survey. *IEEE Transactions on Affective Computing*, 4(1), 15–33.
  **Reasoning.** Whether a state is pleasant or unpleasant is hard to read from the body, so the app does not
  try to.
- Heider, F., & Simmel, M. (1944). An experimental study of apparent behavior. *The American Journal of
  Psychology*, 57(2), 243–259.
- Johansson, G. (1973). Visual perception of biological motion and a model for its analysis. *Perception &
  Psychophysics*, 14(2), 201–211.
  **Reasoning** (both). People read a great deal from movement alone, even of plain shapes or a few points on
  a body. The same layers of evidence therefore apply to anything that can be tracked, not only to faces.

## Pulse from a camera

- Wang, W., den Brinker, A. C., Stuijk, S., & de Haan, G. (2017). Algorithmic principles of remote PPG. *IEEE
  Transactions on Biomedical Engineering*, 64(7), 1479–1491.
  **In use.** The method by which the optional webcam pulse is estimated from small colour changes of the skin.
  It is an estimate for the user's own information, shown with how clear the signal is. The app is not a
  medical device.

## Voice and sound

- Juslin, P. N., & Laukka, P. (2003). Communication of emotions in vocal expression and music performance:
  Different channels, same code? *Psychological Bulletin*, 129(5), 770–814.
  **Reasoning.** A voice carries how aroused someone sounds much better than whether they sound pleased. The
  voice readings are worded and weighted accordingly.
- Cao, H., Cooper, D. G., Keutmann, M. K., Gur, R. C., Nenkova, A., & Verma, R. (2014). CREMA-D: Crowd-sourced
  Emotional Multimodal Actors Dataset. *IEEE Transactions on Affective Computing*, 5(4), 377–390.
  **Basis.** Melos 1, the project's own voice model, was trained on what listeners heard in this collection's
  acted clips. It has been measured on acted speech only.
- Gemmeke, J. F., Ellis, D. P. W., Freedman, D., Jansen, A., Lawrence, W., Moore, R. C., Plakal, M., & Ritter,
  M. (2017). Audio Set: An ontology and human-labeled dataset for audio events. *2017 IEEE International
  Conference on Acoustics, Speech and Signal Processing (ICASSP)*, 776–780.
- Howard, A. G., Zhu, M., Chen, B., Kalenichenko, D., Wang, W., Weyand, T., Andreetto, M., & Adam, H. (2017).
  MobileNets: Efficient convolutional neural networks for mobile vision applications. arXiv:1704.04861.
  **In use** (both). YAMNet, the model that names what an audio interface hears (music, speech, quiet and
  so on), is a network of the second kind trained on the first.

## Finding and recognizing faces

- Wu, W., Peng, H., & Yu, S. (2023). YuNet: A tiny millisecond-level face detector. *Machine Intelligence
  Research*, 20(5), 656–665.
  **In use.** The detector that finds each face and its five points.
- Zhong, Y., Deng, W., Hu, J., Zhao, D., Li, X., & Wen, D. (2021). SFace: Sigmoid-constrained hypersphere loss
  for robust face recognition. *IEEE Transactions on Image Processing*, 30, 2587–2598.
  **Basis.** Opsis 1, the project's own recognizer, was trained further from this model so that a poor picture
  of a face gives the print of a good one.
- Babnik, Ž., Peer, P., & Štruc, V. (2024). eDifFIQA: Towards efficient face image quality assessment based on
  denoising diffusion probabilistic models. *IEEE Transactions on Biometrics, Behavior, and Identity Science*,
  6(4), 458–474.
  **In use.** A quality score for every face picture. It decides which pictures a person is learned from and
  which an expression is read from, and it weights the pictures a live name is made of.
- Malkov, Y. A., & Yashunin, D. A. (2020). Efficient and robust approximate nearest neighbor search using
  hierarchical navigable small world graphs. *IEEE Transactions on Pattern Analysis and Machine Intelligence*,
  42(4), 824–836.
  **In use.** The index over face prints in the optional shared database.
- Fan, R.-E., Chang, K.-W., Hsieh, C.-J., Wang, X.-R., & Lin, C.-J. (2008). LIBLINEAR: A library for large
  linear classification. *Journal of Machine Learning Research*, 9, 1871–1874.
  **In use.** The kind of small linear model, and the way of fitting it, behind the labeling tool's check of
  whether a found region looks like a face.

## Reading expression, face shape and posture

- Jiang, J., & Deng, W. (2023). Boosting facial expression recognition by a semi-supervised progressive
  teacher. *IEEE Transactions on Affective Computing*, 14(3), 2402–2414.
  **In use.** The method by which the expression model was trained. The model gives the seven labels a reading
  starts from (angry, disgust, fearful, happy, neutral, sad, surprised).
- Chen, S., Liu, Y., Gao, X., & Han, Z. (2018). MobileFaceNets: Efficient CNNs for accurate real-time face
  verification on mobile devices. *Chinese Conference on Biometric Recognition (CCBR 2018)*, Lecture Notes in
  Computer Science 10996, 428–438.
  **In use.** The small network that model is built on.
- Li, S., Deng, W., & Du, J. (2017). Reliable crowdsourcing and deep locality-preserving learning for
  expression recognition in the wild. *2017 IEEE Conference on Computer Vision and Pattern Recognition (CVPR)*,
  2852–2861.
  **Reasoning.** The collection of faces (RAF-DB) on which the expression model's published accuracy is
  stated (88.27%). The project does not rest on that figure: it measures the model on posed sessions of its
  own.
- Lugaresi, C., Tang, J., Nash, H., et al. (2019). MediaPipe: A framework for building perception pipelines.
  arXiv:1906.08172.
- Kartynnik, Y., Ablavatski, A., Grishchenko, I., & Grundmann, M. (2019). Real-time facial surface geometry
  from monocular video on mobile GPUs. arXiv:1907.06724.
- Bazarevsky, V., Grishchenko, I., Raveendran, K., Zhu, T., Zhang, F., & Grundmann, M. (2020). BlazePose:
  On-device real-time body pose tracking. arXiv:2006.10204.
- Zhang, F., Bazarevsky, V., Vakunov, A., Tkachenka, A., Sung, G., Chang, C.-L., & Grundmann, M. (2020).
  MediaPipe Hands: On-device real-time hand tracking. arXiv:2006.10214.
  **In use** (all four). The face mesh and its movements, the body points and the hand points, run in the
  page itself. What the app makes of them (talking, blinks, gaze, posture, breathing, a hand on the face) is
  evidence only until it has been measured.

## Describing a photo

- Xiao, B., Wu, H., Xu, W., Dai, X., Hu, H., Lu, Y., Zeng, M., Liu, C., & Yuan, L. (2024). Florence-2: Advancing
  a unified representation for a variety of vision tasks. *2024 IEEE/CVF Conference on Computer Vision and
  Pattern Recognition (CVPR)*, 4818–4829.
  **In use.** The optional built-in model that describes a photo and reads its text, on the computer itself.

## Planned: a measurement from inside (EEG)

Not built. Nothing below has been measured by this project. The articles are named because they set what
such a channel could and could not say.

- Davidson, R. J. (1992). Anterior cerebral asymmetry and the nature of emotion. *Brain and Cognition*, 20(1),
  125–151.
- Coan, J. A., & Allen, J. J. B. (2004). Frontal EEG asymmetry as a moderator and mediator of emotion.
  *Biological Psychology*, 67(1–2), 7–50.
- Harmon-Jones, E., & Gable, P. A. (2018). On the role of asymmetric frontal cortical activity in approach and
  withdrawal motivation: An updated review of the evidence. *Psychophysiology*, 55(1), e12879.
  **Planned** (all three). Frontal asymmetry follows approach and withdrawal better than pleasant and
  unpleasant, so it would be reported as that and not as a measure of mood.
- Koelstra, S., Mühl, C., Soleymani, M., Lee, J.-S., Yazdani, A., Ebrahimi, T., Pun, T., Nijholt, A., & Patras,
  I. (2012). DEAP: A database for emotion analysis using physiological signals. *IEEE Transactions on Affective
  Computing*, 3(1), 18–31.
  **Planned.** Telling states apart from EEG works modestly, and far better within one person than across
  people: one more reason to read each person against themselves.
- Hajcak, G., MacNamara, A., & Olvet, D. M. (2010). Event-related potentials, emotion, and emotion regulation:
  An integrative review. *Developmental Neuropsychology*, 35(2), 129–155.
  **Planned.** The brain's response to an emotional picture in the moments after it appears: one thing such a
  channel could look for.
- Badcock, N. A., Mousikou, P., Mahajan, Y., de Lissa, P., Thie, J., & McArthur, G. (2013). Validation of the
  Emotiv EPOC EEG gaming system for measuring research quality auditory ERPs. *PeerJ*, 1, e38.
  **Planned.** A consumer headset gave responses to sounds close to a laboratory system's. The headset
  considered for this project is a later model, which the project has not checked.
- Whitham, E. M., Pope, K. J., Fitzgibbon, S. P., et al. (2007). Scalp electrical recording during paralysis:
  Quantitative evidence that EEG frequencies above 20 Hz are contaminated by EMG. *Clinical Neurophysiology*,
  118(8), 1877–1888.
  **Planned.** Much of a scalp recording above about 20 Hz is muscle, which any such channel has to allow for.
